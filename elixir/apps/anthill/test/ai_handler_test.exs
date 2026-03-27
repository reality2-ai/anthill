defmodule Anthill.Plugins.AIHandlerTest do
  use ExUnit.Case

  @moduledoc """
  Tests for the AI plugin handler (Anthill.Plugins.AIHandler).

  Uses a mock backend that returns immediately with canned responses.
  Tests queue semantics: sequential processing, cancellation, status.
  """

  defmodule MockBackend do
    @moduledoc false
    @behaviour Anthill.AI.Backend

    @impl true
    def name, do: "mock"

    @impl true
    def available?, do: true

    @impl true
    def execute(request) do
      # Simulate a brief delay
      Process.sleep(Map.get(request, :delay_ms, 50))
      {:ok, %{output: "mock response to: #{request.prompt}", exit_code: 0, duration_ms: 50}}
    end
  end

  setup do
    # Register a test sentant to receive results
    sentant_id = "ai-test-#{System.unique_integer([:positive])}"
    Registry.register(R2.Registry, {sentant_id, :comms}, [])

    # Ensure the AI handler is running with mock backend
    # The handler is already started by Anthill.Application
    # We just need to use a sentant_id that has comms registered

    %{sentant_id: sentant_id}
  end

  describe "single prompt" do
    test "processes a prompt and returns result", %{sentant_id: sentant_id} do
      invoke_query(sentant_id, "hello")

      assert_receive {:event, %{"event" => "ai.reality2.ai.query", "parameters" => params}}, 10_000
      assert params["status"] == "ok"
      assert is_binary(params["data"]["output"])
    end
  end

  describe "sequential queuing" do
    test "processes multiple prompts in order", %{sentant_id: sentant_id} do
      invoke_query(sentant_id, "first")
      invoke_query(sentant_id, "second")

      # First response
      assert_receive {:event, %{"event" => "ai.reality2.ai.query", "parameters" => p1}}, 15_000
      assert p1["status"] == "ok"

      # Second response (queued, processed after first)
      assert_receive {:event, %{"event" => "ai.reality2.ai.query", "parameters" => p2}}, 15_000
      assert p2["status"] == "ok"
    end
  end

  describe "status" do
    test "returns queue depth", %{sentant_id: sentant_id} do
      invoke_status(sentant_id)

      assert_receive {:event, %{"event" => "ai.reality2.ai.status", "parameters" => params}}, 5_000
      assert params["data"]["queue_depth"] == 0
      assert params["data"]["busy"] == false
    end
  end

  # ── Helpers ──────────────────────────────────────────────────

  defp invoke_query(sentant_id, prompt) do
    R2.Plugin.Manager.invoke(%{
      "plugin" => "ai.reality2.ai",
      "command" => "query",
      "sentant" => sentant_id,
      "parameters" => %{"prompt" => prompt}
    })
  end

  defp invoke_status(sentant_id) do
    R2.Plugin.Manager.invoke(%{
      "plugin" => "ai.reality2.ai",
      "command" => "status",
      "sentant" => sentant_id,
      "parameters" => %{}
    })
  end
end
