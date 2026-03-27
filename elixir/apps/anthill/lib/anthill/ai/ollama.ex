defmodule Anthill.AI.Ollama do
  @moduledoc """
  Ollama backend for local LLM inference.

  Uses the Ollama HTTP API (`POST /api/generate`) rather than the CLI,
  which avoids the interactive stdin issue. Requires Ollama running
  locally on the default port (11434).
  """

  @behaviour Anthill.AI.Backend

  @default_model "llama3.2"
  @default_url "http://localhost:11434"

  @impl Anthill.AI.Backend
  def name, do: "ollama"

  @impl Anthill.AI.Backend
  def available? do
    # Check if Ollama API is reachable
    url = api_url() <> "/api/tags"

    case :httpc.request(:get, {String.to_charlist(url), []}, [{:timeout, 2000}], []) do
      {:ok, {{_, 200, _}, _, _}} -> true
      _ -> false
    end
  catch
    _, _ -> false
  end

  @impl Anthill.AI.Backend
  def execute(request) do
    model = Map.get(request, :model, @default_model)
    url = api_url() <> "/api/generate"
    start = System.monotonic_time(:millisecond)

    body =
      Jason.encode!(%{
        model: model,
        prompt: request.prompt,
        system: request.system_prompt || "",
        stream: false
      })

    headers = [{~c"content-type", ~c"application/json"}]
    http_opts = [{:timeout, request.timeout_ms}, {:connect_timeout, 5000}]

    case :httpc.request(
           :post,
           {String.to_charlist(url), headers, ~c"application/json", String.to_charlist(body)},
           http_opts,
           [{:body_format, :binary}]
         ) do
      {:ok, {{_, 200, _}, _resp_headers, resp_body}} ->
        duration = System.monotonic_time(:millisecond) - start

        case Jason.decode(resp_body) do
          {:ok, %{"response" => output}} ->
            {:ok, %{output: output, exit_code: 0, duration_ms: duration}}

          {:ok, %{"error" => error}} ->
            {:error, "ollama error: #{error}"}

          {:error, reason} ->
            {:error, "ollama response parse error: #{inspect(reason)}"}
        end

      {:ok, {{_, status, _}, _, resp_body}} ->
        {:error, "ollama HTTP #{status}: #{String.slice(to_string(resp_body), 0, 500)}"}

      {:error, reason} ->
        {:error, "ollama request failed: #{inspect(reason)}"}
    end
  end

  defp api_url do
    System.get_env("OLLAMA_HOST") || @default_url
  end
end
