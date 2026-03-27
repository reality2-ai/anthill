defmodule Anthill.Plugins.KnowledgeHandler do
  @moduledoc """
  Registers `ai.reality2.knowledge` with the R2 hive plugin manager.

  Provides persistent knowledge graph operations per R2-KNOWLEDGE.
  Each sentant gets an isolated knowledge store scoped to its own
  working directory.

  ## Commands

    * `query` — retrieve relevant knowledge for a topic
    * `update` — store new knowledge from a conversation
    * `add_node` — add an entity to the graph
    * `add_edge` — add a conjecture between entities
    * `update_evidence` — update confidence on an edge
    * `stats` — return graph statistics

  Results are sent back as R2-PLUGIN §2.4 envelope events.
  """

  require Logger

  @plugin_name "ai.reality2.knowledge"

  # In-memory knowledge stores per sentant (will be NIF-backed later)
  @stores_table :anthill_knowledge_stores

  @doc "Register the knowledge plugin with the hive plugin manager."
  @spec register() :: :ok
  def register do
    :ets.new(@stores_table, [:named_table, :set, :public, read_concurrency: true])
    R2.Plugin.Manager.register_plugin(@plugin_name, &handle_invocation/1)
  end

  defp handle_invocation(invocation) do
    sentant_id = Map.get(invocation, "sentant", "")
    command = Map.get(invocation, "command", "")
    params = Map.get(invocation, "parameters", %{})

    store = get_or_create_store(sentant_id)

    {result, data} =
      case command do
        "query" -> {:ok, data} = do_query(store, params); {store, data}
        "update" -> {:ok, new_store, data} = do_update(store, params); {new_store, data}
        "add_node" -> {:ok, data} = do_add_node(store, params); {store, data}
        "add_edge" -> {:ok, data} = do_add_edge(store, params); {store, data}
        "update_evidence" -> {:ok, data} = do_update_evidence(store, params); {store, data}
        "stats" -> {:ok, data} = do_stats(store); {store, data}
        other -> {store, %{"error" => "unknown command: #{other}"}}
      end

    # Save mutated store
    if result != store, do: save_store(sentant_id, result)

    send_result(sentant_id, command, {:ok, data})
  end

  # ── Commands ───────────────────────────────────────────────────

  defp do_query(store, params) do
    topic = Map.get(params, "about", "")
    edges = store.edges

    # Topic-relevant edges (by keyword match)
    relevant =
      if topic != "" do
        topic_lower = String.downcase(topic)
        edges
        |> Enum.filter(fn edge ->
          String.contains?(String.downcase(edge.from), topic_lower) or
            String.contains?(String.downcase(edge.to), topic_lower) or
            String.contains?(String.downcase(edge.relation), topic_lower)
        end)
        |> Enum.sort_by(& &1.confidence, :desc)
        |> Enum.take(5)
      else
        []
      end

    # Recent conversation history (last 10 exchanges, regardless of topic)
    recent =
      edges
      |> Enum.filter(&(&1.relation == "prompted"))
      |> Enum.take(10)

    # Build context: recent conversation first, then relevant knowledge
    history =
      recent
      |> Enum.reverse()
      |> Enum.map(fn e -> "User: #{e.from}\nAssistant: #{e.to}" end)
      |> Enum.join("\n\n")

    knowledge =
      relevant
      |> Enum.map(fn e ->
        "#{e.from} --#{e.relation}--> #{e.to} (confidence: #{Float.round(e.confidence, 2)})"
      end)
      |> Enum.join("\n")

    context = Enum.join([
      if(history != "", do: "Recent conversation:\n#{history}", else: nil),
      if(knowledge != "", do: "Relevant knowledge:\n#{knowledge}", else: nil)
    ] |> Enum.reject(&is_nil/1), "\n\n")

    {:ok, %{
      "context" => context,
      "edge_count" => length(relevant) + length(recent),
      "total_nodes" => length(store.nodes),
      "total_edges" => length(edges)
    }}
  end

  defp do_update(store, params) do
    input = Map.get(params, "input", "")
    output = Map.get(params, "output", "")

    edge = %{
      from: summarise(input),
      to: summarise(output),
      relation: "prompted",
      confidence: 0.6,
      basis: "observed",
      evidence: [],
      created: DateTime.utc_now() |> DateTime.to_iso8601()
    }

    new_store = %{store | edges: [edge | store.edges]}
    {:ok, new_store, %{"stored" => true, "edge_count" => length(new_store.edges)}}
  end

  defp do_add_node(store, params) do
    node = %{
      label: Map.get(params, "label", ""),
      kind: Map.get(params, "kind", "entity"),
      summary: Map.get(params, "summary", ""),
      created: DateTime.utc_now() |> DateTime.to_iso8601()
    }

    new_store = %{store | nodes: [node | store.nodes]}
    save_store_direct(Map.get(params, "_sentant_id", ""), new_store)
    {:ok, %{"added" => node.label}}
  end

  defp do_add_edge(store, params) do
    # Initial confidence from basis per R2-KNOWLEDGE §2.5
    basis = Map.get(params, "basis", "assumed")
    initial_confidence = case basis do
      "observed" -> 0.7
      "told" -> 0.6
      "inferred" -> 0.4
      _ -> 0.3
    end

    edge = %{
      from: Map.get(params, "from", ""),
      to: Map.get(params, "to", ""),
      relation: Map.get(params, "relation", "?"),
      confidence: initial_confidence,
      basis: basis,
      evidence: [],
      created: DateTime.utc_now() |> DateTime.to_iso8601()
    }

    new_store = %{store | edges: [edge | store.edges]}
    save_store_direct(Map.get(params, "_sentant_id", ""), new_store)
    {:ok, %{"added" => "#{edge.from} --#{edge.relation}--> #{edge.to}", "confidence" => initial_confidence}}
  end

  defp do_update_evidence(store, params) do
    from = Map.get(params, "from", "")
    to = Map.get(params, "to", "")
    relation = Map.get(params, "relation", "")
    evidence_type = Map.get(params, "evidence_type", "corroboration")

    # Get base Bayes factor from NIF
    {:ok, bf} = R2.Nif.epistemic_base_bayes_factor(evidence_type)
    reputation = Map.get(params, "source_reputation", 0.5)
    adjusted_bf = R2.Nif.epistemic_reputation_adjusted_bf(bf, reputation)

    new_edges =
      Enum.map(store.edges, fn edge ->
        if edge.from == from and edge.to == to and edge.relation == relation do
          log_odds = R2.Nif.epistemic_to_log_odds(edge.confidence)
          new_log_odds = R2.Nif.epistemic_bayesian_update(log_odds, adjusted_bf)
          new_confidence = R2.Nif.epistemic_to_probability(new_log_odds)

          %{edge |
            confidence: new_confidence,
            evidence: [%{type: evidence_type, bf: adjusted_bf, date: DateTime.utc_now() |> DateTime.to_iso8601()} | edge.evidence]
          }
        else
          edge
        end
      end)

    new_store = %{store | edges: new_edges}
    save_store_direct(Map.get(params, "_sentant_id", ""), new_store)
    {:ok, %{"updated" => "#{from} --#{relation}--> #{to}", "evidence_type" => evidence_type}}
  end

  defp do_stats(store) do
    avg_confidence =
      case store.edges do
        [] -> 0.0
        edges -> Enum.sum(Enum.map(edges, & &1.confidence)) / length(edges)
      end

    {:ok, %{
      "node_count" => length(store.nodes),
      "edge_count" => length(store.edges),
      "avg_confidence" => Float.round(avg_confidence, 3)
    }}
  end

  # ── Store management ───────────────────────────────────────────

  defp get_or_create_store(sentant_id) do
    case :ets.lookup(@stores_table, sentant_id) do
      [{_, store}] -> store
      [] -> %{nodes: [], edges: []}
    end
  end

  defp save_store(sentant_id, store) do
    :ets.insert(@stores_table, {sentant_id, store})
  end

  defp save_store_direct(sentant_id, store) when sentant_id != "" do
    :ets.insert(@stores_table, {sentant_id, store})
  end

  defp save_store_direct(_, _), do: :ok

  defp send_result(sentant_id, command, result) do
    {status, data} =
      case result do
        {:ok, data} -> {"ok", data}
        {:error, reason} -> {"error", %{"error" => reason}}
      end

    # Use command-specific event names so automations can route distinctly
    event_name = "#{@plugin_name}.#{command}"

    event = %{
      "event" => event_name,
      "parameters" => %{
        "plugin" => @plugin_name,
        "command" => command,
        "status" => status,
        "data" => data
      },
      "origin" => :internal,
      "sender" => %{"sentant_id" => "plugin:#{@plugin_name}"}
    }

    case Registry.lookup(R2.Registry, {sentant_id, :comms}) do
      [{pid, _}] -> send(pid, {:event, event})
      [] -> Logger.warning("[knowledge] Sentant #{sentant_id} not found")
    end
  end

  defp summarise(text) when byte_size(text) > 80 do
    String.slice(text, 0, 77) <> "..."
  end

  defp summarise(text), do: text
end
