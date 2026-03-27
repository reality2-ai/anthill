defmodule Anthill.Ant.Conductor do
  @moduledoc """
  Sentant FSM for an ANT.

  Pure state machine — receives events, decides actions, emits plugin
  commands. No I/O, no shared state. Given the same events in the same
  state, always produces the same actions.

  ## States

    * `:idle` — waiting for input
    * `:thinking` — AI plugin is processing a request
  """

  use GenServer
  require Logger

  @type state :: :idle | :thinking

  defstruct [
    :ant_id,
    :config,
    state: :idle,
    plugins: %{},
    pending_reply_to: nil
  ]

  @doc "Start a Conductor for an ANT."
  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts, name: Keyword.fetch!(opts, :name))
  end

  @doc "Send an R2 event to this conductor."
  @spec send_event(GenServer.server(), Anthill.Event.t()) :: :ok
  def send_event(conductor, %Anthill.Event{} = event) do
    GenServer.cast(conductor, {:event, event})
  end

  @impl GenServer
  def init(opts) do
    state = %__MODULE__{
      ant_id: Keyword.fetch!(opts, :ant_id),
      config: Keyword.get(opts, :config, %{}),
      plugins: Keyword.get(opts, :plugins, %{})
    }

    Logger.info("[#{state.ant_id}] Conductor started (state: #{state.state})")
    {:ok, state}
  end

  @impl GenServer
  def handle_cast({:event, event}, state) do
    {:noreply, handle_event(event, state)}
  end

  @impl GenServer
  def handle_info({:event, event}, state) do
    {:noreply, handle_event(event, state)}
  end

  # ── Event handling (the FSM) ──────────────────────────────────

  defp handle_event(%{name: "#ai_request"} = event, %{state: :idle} = s) do
    dispatch_to_plugin(:ai, "query", event.params, s)
    %{s | state: :thinking, pending_reply_to: event.from}
  end

  defp handle_event(%{name: "#ai_request"}, %{state: :thinking} = s) do
    Logger.debug("[#{s.ant_id}] Busy — request dropped (state: thinking)")
    s
  end

  defp handle_event(%{name: "#ai_response"} = event, %{state: :thinking} = s) do
    if s.pending_reply_to do
      "#reply"
      |> Anthill.Event.new(
        %{"output" => event.params["output"], "backend" => event.params["backend"]},
        from: s.ant_id,
        to: s.pending_reply_to
      )
      |> broadcast(s)
    end

    %{s | state: :idle, pending_reply_to: nil}
  end

  defp handle_event(%{name: "#ai_error"} = event, %{state: :thinking} = s) do
    Logger.warning("[#{s.ant_id}] AI error: #{event.params["error"]}")
    %{s | state: :idle, pending_reply_to: nil}
  end

  defp handle_event(%{name: "#ping"} = event, s) do
    "#pong"
    |> Anthill.Event.new(%{}, from: s.ant_id, to: event.from)
    |> broadcast(s)

    s
  end

  defp handle_event(event, s) do
    Logger.debug("[#{s.ant_id}] Unhandled event: #{event.name} (state: #{s.state})")
    s
  end

  # ── Plugin dispatch ────────────────────────────────────────────

  defp dispatch_to_plugin(plugin_key, command, params, state) do
    case Map.fetch(state.plugins, plugin_key) do
      {:ok, name} ->
        GenServer.cast(name, {:command, command, params})

      :error ->
        Logger.warning("[#{state.ant_id}] No plugin registered: #{plugin_key}")
    end
  end

  # ── Event broadcasting ─────────────────────────────────────────

  defp broadcast(%Anthill.Event{} = event, _state) do
    Registry.dispatch(R2.EventBus, :events, fn entries ->
      for {pid, _} <- entries, do: send(pid, {:event, event})
    end)
  end
end
