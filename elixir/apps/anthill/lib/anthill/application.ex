defmodule Anthill.Application do
  @moduledoc """
  Anthill OTP application.

  Depends on `r2_core` for the R2 sentant runtime. On startup,
  registers Anthill-specific plugins with the hive plugin manager:

    * `ai.reality2.ai` — AI backend dispatch (Claude, Ollama, etc.)
  """

  use Application

  @impl Application
  def start(_type, _args) do
    children = [
      {Anthill.Colony, []}
    ]

    result = Supervisor.start_link(children, strategy: :one_for_one, name: Anthill.Supervisor)

    # Register Anthill plugins with the R2 hive plugin manager
    Anthill.Plugins.AIHandler.register(
      backend: System.get_env("ANTHILL_BACKEND", "claude-code"),
      working_dir: System.get_env("ANTHILL_WORKING_DIR", System.tmp_dir!()),
      timeout_ms: 120_000
    )

    Anthill.Plugins.KnowledgeHandler.register()

    result
  end
end
