defmodule AnthillWebWeb.Resolvers.Sentant do
  @moduledoc """
  GraphQL resolvers for sentant operations (R2-GQL).

  Maps queries and mutations to R2.Hive runtime calls.
  Only exposes public information per IPUCO opacity.
  """

  @doc "Get a sentant by ID."
  @spec get(any(), %{id: String.t()}, any()) :: {:ok, map() | nil}
  def get(_parent, %{id: id}, _resolution) do
    {:ok, R2.Hive.get_sentant(id)}
  end

  @doc "List all sentants with optional filtering."
  @spec list(any(), map(), any()) :: {:ok, [map()]}
  def list(_parent, args, _resolution) do
    sentants =
      R2.Hive.list_sentants_full()
      |> maybe_filter_class(args)
      |> maybe_filter_name(args)

    {:ok, sentants}
  end

  @doc "Load a sentant from a definition string."
  @spec load(any(), %{definition: String.t()}, any()) :: {:ok, map()}
  def load(_parent, %{definition: definition}, _resolution) do
    case R2.Hive.load_from_yaml(definition) do
      {:ok, id} when is_binary(id) ->
        {:ok, %{
          success: true,
          sentant: R2.Hive.get_sentant(id),
          errors: []
        }}

      {:ok, ids} when is_list(ids) ->
        {:ok, %{
          success: true,
          sentant: R2.Hive.get_sentant(List.first(ids)),
          errors: []
        }}

      {:error, reason} ->
        {:ok, %{
          success: false,
          sentant: nil,
          errors: [%{code: "LOAD_FAILED", message: inspect(reason)}]
        }}
    end
  end

  @doc "Unload a sentant."
  @spec unload(any(), %{id: String.t()}, any()) :: {:ok, map()}
  def unload(_parent, %{id: id}, _resolution) do
    case R2.Hive.unload(id) do
      :ok ->
        {:ok, %{success: true, unloaded_ids: [id], errors: []}}

      {:error, :not_found} ->
        {:ok, %{
          success: false,
          unloaded_ids: [],
          errors: [%{code: "NOT_FOUND", message: "sentant not found"}]
        }}
    end
  end

  @doc "Send an event to a sentant (origin: GRAPHQL)."
  @spec send_event(any(), map(), any()) :: {:ok, map()}
  def send_event(_parent, %{sentant_id: id, event: event_name} = args, _resolution) do
    event = %{
      "event" => event_name,
      "parameters" => args[:data] || %{},
      "origin" => :graphql,
      "sender" => %{"sentant_id" => "graphql"}
    }

    case R2.Hive.send_event(id, event) do
      :ok ->
        {:ok, %{accepted: true, dispatched_to: 1, errors: []}}

      {:error, :not_found} ->
        {:ok, %{
          accepted: false,
          dispatched_to: 0,
          errors: [%{code: "NOT_FOUND", message: "sentant not found"}]
        }}
    end
  end

  # ── Filters ────────────────────────────────────────────────────

  defp maybe_filter_class(sentants, %{class: class}) when is_binary(class) do
    Enum.filter(sentants, &String.starts_with?(&1.class, class))
  end

  defp maybe_filter_class(sentants, _), do: sentants

  defp maybe_filter_name(sentants, %{name_contains: name}) when is_binary(name) do
    down = String.downcase(name)
    Enum.filter(sentants, &String.contains?(String.downcase(&1.name), down))
  end

  defp maybe_filter_name(sentants, _), do: sentants
end
