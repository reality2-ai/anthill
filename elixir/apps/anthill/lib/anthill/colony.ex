defmodule Anthill.Colony do
  @moduledoc """
  LEGACY: Colony manager — creates and supervises ANTs.

  **Deprecated in favour of `R2.Hive`** which provides R2-compliant
  sentant lifecycle with YAML-driven automations, hive-level plugins,
  and proper IPUCO enforcement.

  This module remains for backward compatibility with the hardcoded
  Conductor FSM and per-sentant plugin model. It will be removed
  once all functionality is migrated to R2.Hive.
  """

  use DynamicSupervisor
  require Logger

  @type ant_config :: %{
          ant_id: String.t(),
          working_dir: String.t(),
          backend: String.t(),
          system_prompt: String.t() | nil,
          timeout_ms: pos_integer()
        }

  @spec start_link(keyword()) :: Supervisor.on_start()
  def start_link(opts) do
    DynamicSupervisor.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl DynamicSupervisor
  def init(_opts) do
    DynamicSupervisor.init(strategy: :one_for_one)
  end

  @doc """
  Start a new ANT with the given configuration.

  ## Required keys

    * `:ant_id` — unique string identifier
    * `:working_dir` — sandboxed filesystem root for this ANT

  ## Optional keys

    * `:backend` — AI backend name (default: `"claude-code"`)
    * `:system_prompt` — default system prompt for the AI backend
    * `:timeout_ms` — AI request timeout in milliseconds (default: `120_000`)

  Returns `{:ok, pid}` or `{:error, reason}`.
  """
  @spec start_ant(map()) :: {:ok, pid()} | {:error, term()}
  def start_ant(config) when is_map(config) do
    with :ok <- validate_config(config) do
      File.mkdir_p!(config.working_dir)
      Logger.info("Starting ANT: #{config.ant_id}")
      DynamicSupervisor.start_child(__MODULE__, {Anthill.Ant.Supervisor, config})
    end
  end

  @doc "Stop an ANT by its identifier."
  @spec stop_ant(String.t()) :: :ok | {:error, :not_found}
  def stop_ant(ant_id) do
    case Registry.lookup(R2.Registry, {ant_id, :supervisor}) do
      [{pid, _}] -> DynamicSupervisor.terminate_child(__MODULE__, pid)
      [] -> {:error, :not_found}
    end
  end

  @doc "List all running ANT identifiers."
  @spec list_ants() :: [String.t()]
  def list_ants do
    __MODULE__
    |> DynamicSupervisor.which_children()
    |> Enum.flat_map(fn {_id, pid, _type, _modules} ->
      case Registry.keys(R2.Registry, pid) do
        [{ant_id, :supervisor}] -> [ant_id]
        _ -> []
      end
    end)
  end

  @doc "Send an R2 event to a specific ANT's conductor."
  @spec send_to(String.t(), Anthill.Event.t()) :: :ok | {:error, :not_found}
  def send_to(ant_id, %Anthill.Event{} = event) do
    case Registry.lookup(R2.Registry, {ant_id, :conductor}) do
      [{pid, _}] ->
        Anthill.Ant.Conductor.send_event(pid, event)
        :ok

      [] ->
        {:error, :not_found}
    end
  end

  defp validate_config(%{ant_id: ant_id, working_dir: working_dir})
       when is_binary(ant_id) and is_binary(working_dir) do
    :ok
  end

  defp validate_config(_config) do
    {:error, {:invalid_config, "must include string :ant_id and :working_dir"}}
  end
end
