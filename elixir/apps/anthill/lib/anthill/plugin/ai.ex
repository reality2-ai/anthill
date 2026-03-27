defmodule Anthill.Plugin.AI do
  @moduledoc """
  AI plugin for ANTs.

  Receives command events from the Conductor, dispatches to the configured
  AI backend (Claude, Ollama, etc.), and returns the result as an event.
  The AI subprocess is sandboxed to the ANT's working directory.
  """

  use GenServer
  require Logger

  @behaviour Anthill.Plugin

  defstruct [:ant_id, :working_dir, :backend, :system_prompt, :timeout_ms, :conductor]

  @doc "Start the AI plugin for an ANT."
  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts, name: Keyword.fetch!(opts, :name))
  end

  @impl GenServer
  def init(opts) do
    ant_id = Keyword.fetch!(opts, :ant_id)

    case resolve_backend(Keyword.get(opts, :backend, "claude-code")) do
      {:ok, backend} ->
        state = %__MODULE__{
          ant_id: ant_id,
          working_dir: Keyword.fetch!(opts, :working_dir),
          backend: backend,
          system_prompt: Keyword.get(opts, :system_prompt),
          timeout_ms: Keyword.get(opts, :timeout_ms, 120_000),
          conductor: Keyword.get(opts, :conductor)
        }

        Logger.info("[#{ant_id}] AI plugin started with backend: #{backend.name()}")
        {:ok, state}

      {:error, reason} ->
        {:stop, reason}
    end
  end

  @impl Anthill.Plugin
  def handle_command("query", %{"prompt" => prompt} = params, state) do
    request = %{
      prompt: prompt,
      system_prompt: Map.get(params, "system_prompt", state.system_prompt),
      working_dir: state.working_dir,
      timeout_ms: Map.get(params, "timeout_ms", state.timeout_ms)
    }

    execute_async(request, state)
    {:reply, %{"status" => "dispatched"}, state}
  end

  @impl Anthill.Plugin
  def handle_command(cmd, _params, state) do
    Logger.warning("[#{state.ant_id}] AI plugin: unknown command #{cmd}")
    {:reply, %{"error" => "unknown_command"}, state}
  end

  @impl Anthill.Plugin
  def handle_event(_event, state), do: {:ok, state}

  @impl GenServer
  def handle_cast({:command, cmd, params}, state) do
    case handle_command(cmd, params, state) do
      {:reply, _reply, new_state} -> {:noreply, new_state}
      {:noreply, new_state} -> {:noreply, new_state}
    end
  end

  @impl GenServer
  def handle_call({:command, cmd, params}, _from, state) do
    case handle_command(cmd, params, state) do
      {:reply, reply, new_state} -> {:reply, reply, new_state}
      {:noreply, new_state} -> {:reply, :ok, new_state}
    end
  end

  # ── Private ────────────────────────────────────────────────────

  defp execute_async(request, state) do
    %{ant_id: ant_id, backend: backend, conductor: conductor} = state

    Task.start_link(fn ->
      event =
        case backend.execute(request) do
          {:ok, response} ->
            Anthill.Event.new("#ai_response", %{
              "output" => response.output,
              "duration_ms" => response.duration_ms,
              "backend" => backend.name()
            }, from: {:plugin, :ai})

          {:error, reason} ->
            Logger.warning("[#{ant_id}] AI error: #{reason}")

            Anthill.Event.new("#ai_error", %{
              "error" => reason,
              "backend" => backend.name()
            }, from: {:plugin, :ai})
        end

      if conductor, do: GenServer.cast(conductor, {:event, event})
    end)
  end

  defp resolve_backend(name) when is_atom(name), do: {:ok, name}

  defp resolve_backend(name) when is_binary(name) do
    case name do
      n when n in ["claude-code", "claude"] -> {:ok, Anthill.AI.Claude}
      n when n in ["ollama"] -> {:ok, Anthill.AI.Ollama}
      "ollama/" <> _model -> {:ok, Anthill.AI.Ollama}
      other -> {:error, {:unknown_backend, other}}
    end
  end
end
