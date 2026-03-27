defmodule Anthill.Ant.Supervisor do
  @moduledoc """
  Supervision tree for a single ANT.

  Each ANT is fully isolated — its own conductor, its own plugins,
  its own working directory. Plugin crash restarts only that plugin.
  Conductor crash restarts the conductor without affecting plugins.
  """

  use Supervisor

  def start_link(config) do
    Supervisor.start_link(__MODULE__, config, name: via(config.ant_id))
  end

  @impl Supervisor
  def init(config) do
    ant_id = config.ant_id
    conductor_name = {:via, Registry, {R2.Registry, {ant_id, :conductor}}}
    ai_plugin_name = {:via, Registry, {R2.Registry, {ant_id, :plugin_ai}}}
    file_plugin_name = {:via, Registry, {R2.Registry, {ant_id, :plugin_file}}}

    children = [
      # File Plugin — filesystem sandbox, started first
      {Anthill.Plugin.File, [
        name: file_plugin_name,
        ant_id: ant_id,
        working_dir: config.working_dir
      ]},

      # AI Plugin
      {Anthill.Plugin.AI, [
        name: ai_plugin_name,
        ant_id: ant_id,
        working_dir: config.working_dir,
        backend: Map.get(config, :backend, "claude-code"),
        system_prompt: Map.get(config, :system_prompt),
        timeout_ms: Map.get(config, :timeout_ms, 120_000),
        conductor: conductor_name
      ]},

      # Conductor FSM
      {Anthill.Ant.Conductor, [
        name: conductor_name,
        ant_id: ant_id,
        config: config,
        plugins: %{
          ai: ai_plugin_name,
          file: file_plugin_name
        }
      ]}
    ]

    Supervisor.init(children, strategy: :one_for_one)
  end

  defp via(ant_id) do
    {:via, Registry, {R2.Registry, {ant_id, :supervisor}}}
  end
end
