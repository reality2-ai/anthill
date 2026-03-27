defmodule Anthill.Plugins.AIHandler do
  @moduledoc """
  Stateful AI plugin — `ai.reality2.ai`.

  Hive-level plugin (R2-PLUGIN) that manages a per-sentant task queue.
  Prompts are queued and processed sequentially per sentant. While one
  prompt is running, subsequent prompts queue up and execute in order.

  Ported from the Rust `ai_plugin.rs` / `ai_worker.rs` task model:
  - Per-sentant task queue
  - Follow-up messages into running tasks
  - Task state tracking (running/completed/cancelled/failed)
  - Backend resolution (Claude, Ollama, etc.)

  ## Commands

    * `query` — queue a prompt for processing
    * `cancel` — cancel the current running task
    * `status` — return queue depth and current task state
    * `followup` — inject context into the current running task's next prompt
  """

  use GenServer
  require Logger

  @plugin_name "ai.reality2.ai"

  defmodule TaskEntry do
    @moduledoc false
    defstruct [:invocation, :task_id]
  end

  defmodule SentantQueue do
    @moduledoc false
    defstruct queue: :queue.new(),
              busy: false,
              current_task_pid: nil,
              current_task_id: nil,
              task_counter: 0,
              follow_ups: []
  end

  defstruct [
    :default_backend,
    :default_working_dir,
    :default_timeout,
    sentants: %{}
  ]

  # ── Public API ─────────────────────────────────────────────────

  @doc "Register the AI plugin with the hive plugin manager."
  @spec register(keyword()) :: :ok
  def register(opts) do
    case GenServer.whereis(__MODULE__) do
      nil ->
        {:ok, _} = GenServer.start_link(__MODULE__, opts, name: __MODULE__)

      _pid ->
        :ok
    end

    :ok
  end

  # ── GenServer ──────────────────────────────────────────────────

  @impl GenServer
  def init(opts) do
    backend = Keyword.get(opts, :backend, "claude-code")
    working_dir = Keyword.get(opts, :working_dir, System.tmp_dir!())
    timeout_ms = Keyword.get(opts, :timeout_ms, 120_000)

    R2.Plugin.Manager.register_plugin(@plugin_name, fn invocation ->
      GenServer.cast(__MODULE__, {:invoke, invocation})
    end)

    Logger.info("[ai_handler] Started (default backend: #{backend})")

    {:ok, %__MODULE__{
      default_backend: backend,
      default_working_dir: working_dir,
      default_timeout: timeout_ms
    }}
  end

  @impl GenServer
  def handle_cast({:invoke, invocation}, state) do
    sentant_id = Map.get(invocation, "sentant", "")
    command = Map.get(invocation, "command", "query")

    new_state =
      case command do
        "query" -> enqueue(sentant_id, invocation, state)
        "cancel" -> cancel_task(sentant_id, state)
        "status" -> send_status(sentant_id, invocation, state)
        "followup" -> add_followup(sentant_id, invocation, state)
        _ -> state
      end

    {:noreply, new_state}
  end

  @impl GenServer
  def handle_info({:task_complete, sentant_id, result_event}, state) do
    deliver_to_sentant(sentant_id, result_event)

    sq = get_sq(state, sentant_id)
    new_sq = %{sq | busy: false, current_task_pid: nil, current_task_id: nil, follow_ups: []}
    new_state = put_sq(state, sentant_id, new_sq)

    {:noreply, maybe_process_next(sentant_id, new_state)}
  end

  @impl GenServer
  def handle_info({:task_progress, sentant_id, progress_event}, state) do
    deliver_to_sentant(sentant_id, progress_event)
    {:noreply, state}
  end

  # ── Queue management ───────────────────────────────────────────

  defp enqueue(sentant_id, invocation, state) do
    sq = get_sq(state, sentant_id)
    task_id = sq.task_counter + 1
    entry = %TaskEntry{invocation: invocation, task_id: task_id}
    new_sq = %{sq | queue: :queue.in(entry, sq.queue), task_counter: task_id}

    depth = :queue.len(new_sq.queue) + if(new_sq.busy, do: 1, else: 0)
    if depth > 1 do
      deliver_to_sentant(sentant_id, status_event("queued", %{
        "task_id" => task_id,
        "queue_depth" => depth
      }))
    end

    updated_state = put_sq(state, sentant_id, new_sq)
    maybe_process_next(sentant_id, updated_state)
  end

  defp maybe_process_next(sentant_id, state) do
    sq = get_sq(state, sentant_id)

    if sq.busy do
      state
    else
      case :queue.out(sq.queue) do
        {:empty, _} ->
          state

        {{:value, entry}, new_queue} ->
          new_sq = %{sq |
            queue: new_queue,
            busy: true,
            current_task_id: entry.task_id
          }

          pid = execute_task(sentant_id, entry, state)
          put_sq(state, sentant_id, %{new_sq | current_task_pid: pid})
      end
    end
  end

  defp cancel_task(sentant_id, state) do
    sq = get_sq(state, sentant_id)

    if sq.current_task_pid && Process.alive?(sq.current_task_pid) do
      Process.exit(sq.current_task_pid, :kill)
      deliver_to_sentant(sentant_id, status_event("cancelled", %{
        "task_id" => sq.current_task_id
      }))
    end

    new_sq = %{sq | busy: false, current_task_pid: nil, current_task_id: nil}
    updated_state = put_sq(state, sentant_id, new_sq)
    maybe_process_next(sentant_id, updated_state)
  end

  defp send_status(sentant_id, _invocation, state) do
    sq = get_sq(state, sentant_id)

    deliver_to_sentant(sentant_id, %{
      "event" => "#{@plugin_name}.status",
      "parameters" => %{
        "plugin" => @plugin_name,
        "command" => "status",
        "status" => "ok",
        "data" => %{
          "busy" => sq.busy,
          "current_task_id" => sq.current_task_id,
          "queue_depth" => :queue.len(sq.queue),
          "follow_ups" => length(sq.follow_ups)
        }
      },
      "origin" => :internal,
      "sender" => %{"sentant_id" => "plugin:#{@plugin_name}"}
    })

    state
  end

  defp add_followup(sentant_id, invocation, state) do
    sq = get_sq(state, sentant_id)
    message = get_in(invocation, ["parameters", "message"]) || ""
    new_sq = %{sq | follow_ups: sq.follow_ups ++ [message]}
    put_sq(state, sentant_id, new_sq)
  end

  # ── Task execution ─────────────────────────────────────────────

  defp execute_task(sentant_id, entry, state) do
    invocation = entry.invocation
    params = Map.get(invocation, "parameters", %{})
    command = Map.get(invocation, "command", "query")

    prompt = Map.get(params, "prompt", "")
    backend_name = Map.get(params, "backend", state.default_backend)
    working_dir = Map.get(params, "working_dir", state.default_working_dir)
    timeout_ms = Map.get(params, "timeout_ms", state.default_timeout)
    system_prompt = Map.get(params, "system_prompt")

    backend = resolve_backend(backend_name)
    handler = self()

    {:ok, pid} = Task.start_link(fn ->
      request = %{
        prompt: prompt,
        system_prompt: system_prompt,
        working_dir: working_dir,
        timeout_ms: timeout_ms
      }

      result_event =
        case backend.execute(request) do
          {:ok, response} ->
            %{
              "event" => "#{@plugin_name}.#{command}",
              "parameters" => %{
                "plugin" => @plugin_name,
                "command" => command,
                "status" => "ok",
                "data" => %{
                  "output" => response.output,
                  "duration_ms" => response.duration_ms,
                  "backend" => backend.name(),
                  "task_id" => entry.task_id
                }
              },
              "origin" => :internal,
              "sender" => %{"sentant_id" => "plugin:#{@plugin_name}"}
            }

          {:error, reason} ->
            %{
              "event" => "#{@plugin_name}.#{command}",
              "parameters" => %{
                "plugin" => @plugin_name,
                "command" => command,
                "status" => "error",
                "error" => reason,
                "data" => %{"task_id" => entry.task_id}
              },
              "origin" => :internal,
              "sender" => %{"sentant_id" => "plugin:#{@plugin_name}"}
            }
        end

      send(handler, {:task_complete, sentant_id, result_event})
    end)

    pid
  end

  # ── Helpers ────────────────────────────────────────────────────

  defp get_sq(state, sentant_id) do
    Map.get(state.sentants, sentant_id, %SentantQueue{})
  end

  defp put_sq(state, sentant_id, sq) do
    %{state | sentants: Map.put(state.sentants, sentant_id, sq)}
  end

  defp deliver_to_sentant(sentant_id, event) do
    case Registry.lookup(R2.Registry, {sentant_id, :comms}) do
      [{pid, _}] -> send(pid, {:event, event})
      [] -> Logger.warning("[ai_handler] Sentant #{sentant_id} not found")
    end
  end

  defp status_event(type, data) do
    %{
      "event" => "#{@plugin_name}.status",
      "parameters" => %{
        "plugin" => @plugin_name,
        "command" => "status",
        "status" => "ok",
        "data" => Map.put(data, "type", type)
      },
      "origin" => :internal,
      "sender" => %{"sentant_id" => "plugin:#{@plugin_name}"}
    }
  end

  defp resolve_backend(name) do
    case name do
      n when n in ["claude-code", "claude"] -> Anthill.AI.Claude
      n when n in ["ollama"] -> Anthill.AI.Ollama
      "ollama/" <> _model -> Anthill.AI.Ollama
      _ -> Anthill.AI.Claude
    end
  end
end
