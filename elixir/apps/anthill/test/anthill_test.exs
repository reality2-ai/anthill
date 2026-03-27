defmodule AnthillTest do
  use ExUnit.Case

  setup do
    tmp = Path.join(System.tmp_dir!(), "anthill_test_#{:rand.uniform(999_999)}")
    File.mkdir_p!(tmp)
    on_exit(fn -> File.rm_rf!(tmp) end)
    %{tmp: tmp}
  end

  # ── Colony ──────────────────────────────────────────────────────

  test "colony starts and lists ants", %{tmp: tmp} do
    assert Anthill.Colony.list_ants() == []

    {:ok, _pid} = Anthill.Colony.start_ant(%{
      ant_id: "test-ant",
      working_dir: Path.join(tmp, "test-ant"),
      backend: "claude-code"
    })

    ants = Anthill.Colony.list_ants()
    assert "test-ant" in ants

    :ok = Anthill.Colony.stop_ant("test-ant")
    assert Anthill.Colony.list_ants() == []
  end

  test "stop non-existent ant returns error" do
    assert {:error, :not_found} = Anthill.Colony.stop_ant("no-such-ant")
  end

  test "send to non-existent ant returns error" do
    event = Anthill.Event.new("#ping")
    assert {:error, :not_found} = Anthill.Colony.send_to("no-such-ant", event)
  end

  # ── Events ──────────────────────────────────────────────────────

  test "event creation uses R2-FNV hash" do
    event = Anthill.Event.new("#ping", %{"seq" => 1})
    assert event.name == "#ping"
    assert is_integer(event.event_hash)
    assert event.params == %{"seq" => 1}
  end

  # ── Conductor FSM ───────────────────────────────────────────────

  test "conductor handles ping/pong", %{tmp: tmp} do
    {:ok, _pid} = Anthill.Colony.start_ant(%{
      ant_id: "pinger",
      working_dir: Path.join(tmp, "pinger"),
      backend: "claude-code"
    })

    Registry.register(R2.EventBus, :events, [])

    ping = Anthill.Event.new("#ping", %{}, from: self())
    :ok = Anthill.Colony.send_to("pinger", ping)

    assert_receive {:event, %Anthill.Event{name: "#pong"}}, 1000

    :ok = Anthill.Colony.stop_ant("pinger")
  end

  # ── File Plugin Sandbox ─────────────────────────────────────────

  test "file plugin allows reads within sandbox", %{tmp: tmp} do
    root = Path.join(tmp, "file-ant")
    File.mkdir_p!(root)
    File.write!(Path.join(root, "test.txt"), "hello")

    assert {:ok, full} = Anthill.Plugin.File.resolve("test.txt", root)
    assert full == Path.join(root, "test.txt")
  end

  test "file plugin rejects path traversal", %{tmp: tmp} do
    root = Path.join(tmp, "file-ant")
    File.mkdir_p!(root)

    assert {:error, "access denied: path escapes sandbox"} =
             Anthill.Plugin.File.resolve("../other-ant/secret.txt", root)
  end

  test "file plugin rejects absolute paths", %{tmp: tmp} do
    root = Path.join(tmp, "file-ant")
    File.mkdir_p!(root)

    assert {:error, "absolute paths not allowed"} =
             Anthill.Plugin.File.resolve("/etc/passwd", root)
  end

  test "file plugin allows nested paths within sandbox", %{tmp: tmp} do
    root = Path.join(tmp, "file-ant")
    File.mkdir_p!(root)

    assert {:ok, full} = Anthill.Plugin.File.resolve("memory/graphs/topic.cbor", root)
    assert String.starts_with?(full, root)
  end

  test "file plugin read/write through ANT", %{tmp: tmp} do
    {:ok, _pid} = Anthill.Colony.start_ant(%{
      ant_id: "file-test",
      working_dir: Path.join(tmp, "file-test"),
      backend: "claude-code"
    })

    # Get the file plugin PID
    [{file_pid, _}] = Registry.lookup(R2.Registry, {"file-test", :plugin_file})

    # Write a file
    reply = GenServer.call(file_pid, {:command, "write", %{"path" => "test.txt", "content" => "hello R2"}})
    assert reply["status"] == "ok"

    # Read it back
    reply = GenServer.call(file_pid, {:command, "read", %{"path" => "test.txt"}})
    assert reply["content"] == "hello R2"

    # List directory
    reply = GenServer.call(file_pid, {:command, "list", %{"path" => "."}})
    assert "test.txt" in reply["entries"]

    # Attempt escape
    reply = GenServer.call(file_pid, {:command, "read", %{"path" => "../../etc/passwd"}})
    assert reply["error"] == "access denied: path escapes sandbox"

    :ok = Anthill.Colony.stop_ant("file-test")
  end

  # ── Config Loading ──────────────────────────────────────────────

  test "config loads ant.toml files", %{tmp: tmp} do
    # Create colony structure
    ant_dir = Path.join([tmp, "ants", "alfred"])
    File.mkdir_p!(ant_dir)

    File.write!(Path.join(ant_dir, "ant.toml"), """
    [claude]
    backend = "ollama"
    system_prompt = "You are Alfred."
    timeout_secs = 60
    """)

    configs = Anthill.Config.load(tmp)
    assert length(configs) == 1

    [config] = configs
    assert config.ant_id == "alfred"
    assert config.backend == "ollama"
    assert config.system_prompt == "You are Alfred."
    assert config.timeout_ms == 60_000
    assert String.ends_with?(config.working_dir, "alfred/working")
  end

  test "config skips directories without ant.toml", %{tmp: tmp} do
    File.mkdir_p!(Path.join([tmp, "ants", "empty-dir"]))
    configs = Anthill.Config.load(tmp)
    assert configs == []
  end
end
