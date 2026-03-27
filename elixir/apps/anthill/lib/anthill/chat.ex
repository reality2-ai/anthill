defmodule Anthill.Chat do
  @moduledoc """
  Convenience module for interactive ANT communication.

  Provides synchronous prompt/response for IEx testing and simple
  integrations. Registers on the event bus, sends an `#ai_request`
  event, and waits for the `#reply`.
  """

  @default_timeout 120_000

  @doc """
  Send a prompt to an ANT and wait for the response.

  Returns `{:ok, output}` or `{:error, reason}`.

  ## Options

    * `:timeout` — maximum wait in milliseconds (default: #{@default_timeout})

  ## Examples

      iex> Anthill.Chat.ask("alfred", "What is R2?")
      {:ok, "R2 is a decentralised mesh networking platform..."}
  """
  @spec ask(String.t(), String.t(), keyword()) :: {:ok, String.t()} | {:error, term()}
  def ask(ant_id, prompt, opts \\ []) do
    timeout = Keyword.get(opts, :timeout, @default_timeout)
    Registry.register(R2.EventBus, :events, [])

    event = Anthill.Event.new("#ai_request", %{"prompt" => prompt}, from: self())

    case Anthill.Colony.send_to(ant_id, event) do
      :ok ->
        result = wait_for_reply(timeout)
        Registry.unregister(R2.EventBus, :events)
        result

      {:error, _} = error ->
        Registry.unregister(R2.EventBus, :events)
        error
    end
  end

  @doc """
  Start a temporary ANT, send a prompt, return the response, and stop the ANT.

  Useful for quick one-shot testing from IEx.

  ## Options

    * `:ant_id` — identifier (default: random)
    * `:backend` — AI backend (default: `"claude-code"`)
    * `:working_dir` — sandbox directory (default: system temp)
    * `:timeout` — maximum wait (default: #{@default_timeout})
  """
  @spec quick(String.t(), keyword()) :: {:ok, String.t()} | {:error, term()}
  def quick(prompt, opts \\ []) do
    ant_id = Keyword.get(opts, :ant_id, "quick-#{System.unique_integer([:positive])}")
    backend = Keyword.get(opts, :backend, "claude-code")
    working_dir = Keyword.get(opts, :working_dir, Path.join(System.tmp_dir!(), ant_id))

    with {:ok, _pid} <-
           Anthill.Colony.start_ant(%{
             ant_id: ant_id,
             working_dir: working_dir,
             backend: backend
           }) do
      result = ask(ant_id, prompt, opts)
      Anthill.Colony.stop_ant(ant_id)
      result
    end
  end

  defp wait_for_reply(timeout) do
    receive do
      {:event, %Anthill.Event{name: "#reply", params: params}} ->
        {:ok, params["output"]}

      {:event, %Anthill.Event{name: "#ai_error", params: params}} ->
        {:error, params["error"]}
    after
      timeout -> {:error, :timeout}
    end
  end
end
