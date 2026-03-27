defmodule AnthillWebWeb.AntChannel do
  @moduledoc """
  Phoenix Channel for real-time sentant interaction.

  Clients join `sentant:<sentant_id>` to send events and receive
  responses in real time. The channel registers as a "virtual sentant"
  on the R2 registry so it can receive `@sender` replies.

  ## Incoming messages

    * `"prompt"` — `%{"text" => "..."}` — send prompt to ANT
    * `"event"` — `%{"event" => "...", "parameters" => %{}}` — send any R2 event
    * `"ping"` — health check

  ## Outgoing pushes

    * `"response"` — `%{"output" => "...", "backend" => "..."}` — AI response
    * `"event"` — any R2 event from the sentant
    * `"status"` — `%{"state" => "idle|thinking|..."}` — FSM state change
    * `"knowledge"` — `%{...}` — knowledge plugin results
  """

  use Phoenix.Channel
  require Logger

  @impl Phoenix.Channel
  def join("sentant:" <> sentant_id, _params, socket) do
    case R2.Hive.get_sentant(sentant_id) do
      nil ->
        {:error, %{"reason" => "sentant_not_found"}}

      sentant_info ->
        channel_sentant_id = "web:#{sentant_id}:#{inspect(self())}"
        Registry.register(R2.Registry, {channel_sentant_id, :comms}, [])
        Registry.register(R2.EventBus, :events, [])

        socket =
          socket
          |> assign(:sentant_id, sentant_id)
          |> assign(:channel_sentant_id, channel_sentant_id)

        # Send chat history from knowledge store
        send(self(), :send_history)

        Logger.info("[channel] Client joined sentant:#{sentant_id} (#{sentant_info.name})")
        {:ok, %{"sentant_id" => sentant_id, "name" => sentant_info.name}, socket}
    end
  end

  @impl Phoenix.Channel
  def handle_in("prompt", %{"text" => text}, socket) do
    sentant_id = socket.assigns.sentant_id
    channel_id = socket.assigns.channel_sentant_id

    event = %{
      "event" => "prompt",
      "parameters" => %{"text" => text},
      "origin" => :external,
      "sender" => %{
        "sentant_id" => channel_id,
        "sentant_name" => "web-client"
      }
    }

    case R2.Hive.send_event(sentant_id, event) do
      :ok ->
        push(socket, "status", %{"state" => "thinking"})
        {:noreply, socket}

      {:error, :not_found} ->
        push(socket, "error", %{"message" => "sentant not found"})
        {:noreply, socket}
    end
  end

  @impl Phoenix.Channel
  def handle_in("event", %{"event" => event_name} = params, socket) do
    sentant_id = socket.assigns.sentant_id
    channel_id = socket.assigns.channel_sentant_id

    event = %{
      "event" => event_name,
      "parameters" => Map.get(params, "parameters", %{}),
      "origin" => :external,
      "sender" => %{
        "sentant_id" => channel_id,
        "sentant_name" => "web-client"
      }
    }

    R2.Hive.send_event(sentant_id, event)
    {:noreply, socket}
  end

  @impl Phoenix.Channel
  def handle_in("knowledge_stats", _params, socket) do
    # Send as proper R2 event through the sentant
    sentant_id = socket.assigns.sentant_id
    channel_id = socket.assigns.channel_sentant_id

    event = %{
      "event" => "knowledge_stats",
      "parameters" => %{},
      "origin" => :external,
      "sender" => %{"sentant_id" => channel_id}
    }

    R2.Hive.send_event(sentant_id, event)
    {:noreply, socket}
  end

  @impl Phoenix.Channel
  def handle_in("ping", _params, socket) do
    push(socket, "pong", %{})
    {:noreply, socket}
  end

  @impl Phoenix.Channel
  def handle_in(_, _, socket), do: {:noreply, socket}

  # ── Chat history on join ────────────────────────────────────────

  @impl Phoenix.Channel
  def handle_info(:send_history, socket) do
    sentant_id = socket.assigns.sentant_id

    history =
      case :ets.lookup(:anthill_knowledge_stores, sentant_id) do
        [{_, store}] ->
          store.edges
          |> Enum.filter(&(&1.relation == "prompted"))
          |> Enum.reverse()
          |> Enum.flat_map(fn edge ->
            [
              %{"role" => "user", "text" => edge.from},
              %{"role" => "ant", "text" => edge.to}
            ]
          end)

        [] ->
          []
      end

    if history != [] do
      push(socket, "history", %{"messages" => history})
    end

    {:noreply, socket}
  end

  # ── Incoming R2 events (from sentant via comms) ────────────────

  @impl Phoenix.Channel
  def handle_info({:event, %{"event" => "response", "parameters" => params}}, socket) do
    push(socket, "response", %{
      "output" => Map.get(params, "output", ""),
      "backend" => Map.get(params, "backend", "")
    })
    push(socket, "status", %{"state" => "idle"})
    {:noreply, socket}
  end

  @impl Phoenix.Channel
  def handle_info({:event, %{"event" => "pong"}}, socket) do
    push(socket, "pong", %{})
    {:noreply, socket}
  end

  @impl Phoenix.Channel
  def handle_info({:event, %{"event" => "knowledge_stats_result", "parameters" => params}}, socket) do
    push(socket, "knowledge", params)
    {:noreply, socket}
  end

  @impl Phoenix.Channel
  def handle_info({:event, %{"event" => event_name, "parameters" => params}}, socket) do
    push(socket, "event", %{"event" => event_name, "parameters" => params})
    {:noreply, socket}
  end

  @impl Phoenix.Channel
  def handle_info(_, socket), do: {:noreply, socket}

  @impl Phoenix.Channel
  def terminate(_reason, socket) do
    Logger.info("[channel] Client left sentant:#{socket.assigns[:sentant_id]}")
    :ok
  end
end
