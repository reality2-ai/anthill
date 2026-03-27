defmodule Anthill.AI.Backend do
  @moduledoc """
  Behaviour for AI backends.

  Any AI engine (Claude, Ollama, Gemini, Codex, a shell script, a human)
  implements this behaviour. The sentant's AI plugin dispatches to whatever
  backend is configured — the sentant never knows which AI is running.

  Backends are stateless request/response. Long-running conversations are
  managed by the AI plugin, not the backend.
  """

  @type request :: %{
          prompt: String.t(),
          system_prompt: String.t() | nil,
          working_dir: String.t(),
          timeout_ms: pos_integer()
        }

  @type response :: %{
          output: String.t(),
          exit_code: integer(),
          duration_ms: non_neg_integer()
        }

  @doc """
  Execute a prompt and return the response.

  The backend MUST restrict its subprocess to the given `working_dir`.
  It MUST NOT access files outside that directory.
  """
  @callback execute(request()) :: {:ok, response()} | {:error, String.t()}

  @doc """
  Return a human-readable name for this backend (e.g. "claude-code", "ollama/llama3").
  """
  @callback name() :: String.t()

  @doc """
  Check if this backend is available (binary exists, API key set, etc.).
  """
  @callback available?() :: boolean()
end
