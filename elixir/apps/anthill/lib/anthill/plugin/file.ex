defmodule Anthill.Plugin.File do
  @moduledoc """
  File management plugin — enforces per-ANT filesystem isolation.

  Every file operation is mediated through this plugin. The AI subprocess
  and other plugins MUST NOT access the filesystem directly. This plugin
  canonicalises paths and rejects any access outside the ANT's root.

  Per R2-KNOWLEDGE §12.1: sentants MUST NOT access other sentants'
  storage directly.
  """

  use GenServer
  require Logger

  @behaviour Anthill.Plugin

  defstruct [:ant_id, :root]

  @doc "Start the File plugin for an ANT."
  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts, name: Keyword.fetch!(opts, :name))
  end

  @impl GenServer
  def init(opts) do
    ant_id = Keyword.fetch!(opts, :ant_id)
    root = opts |> Keyword.fetch!(:working_dir) |> Path.expand()
    File.mkdir_p!(root)

    Logger.info("[#{ant_id}] File plugin started (root: #{root})")
    {:ok, %__MODULE__{ant_id: ant_id, root: root}}
  end

  # ── Plugin commands ─────────────────────────────────────────────

  @impl Anthill.Plugin
  def handle_command("read", %{"path" => path}, state) do
    with {:ok, full} <- resolve(path, state.root),
         {:ok, content} <- File.read(full) do
      {:reply, %{"content" => content}, state}
    else
      {:error, reason} -> {:reply, %{"error" => format_error(reason)}, state}
    end
  end

  @impl Anthill.Plugin
  def handle_command("write", %{"path" => path, "content" => content}, state) do
    with {:ok, full} <- resolve(path, state.root) do
      File.mkdir_p!(Path.dirname(full))

      case File.write(full, content) do
        :ok -> {:reply, %{"status" => "ok", "path" => path}, state}
        {:error, reason} -> {:reply, %{"error" => format_error(reason)}, state}
      end
    else
      {:error, reason} -> {:reply, %{"error" => format_error(reason)}, state}
    end
  end

  @impl Anthill.Plugin
  def handle_command("list", %{"path" => path}, state) do
    with {:ok, full} <- resolve(path, state.root),
         {:ok, entries} <- File.ls(full) do
      {:reply, %{"entries" => entries}, state}
    else
      {:error, reason} -> {:reply, %{"error" => format_error(reason)}, state}
    end
  end

  @impl Anthill.Plugin
  def handle_command("exists", %{"path" => path}, state) do
    case resolve(path, state.root) do
      {:ok, full} -> {:reply, %{"exists" => File.exists?(full)}, state}
      {:error, reason} -> {:reply, %{"error" => format_error(reason)}, state}
    end
  end

  @impl Anthill.Plugin
  def handle_command("delete", %{"path" => path}, state) do
    with {:ok, full} <- resolve(path, state.root),
         :ok <- File.rm(full) do
      {:reply, %{"status" => "ok"}, state}
    else
      {:error, reason} -> {:reply, %{"error" => format_error(reason)}, state}
    end
  end

  @impl Anthill.Plugin
  def handle_command("mkdir", %{"path" => path}, state) do
    with {:ok, full} <- resolve(path, state.root),
         :ok <- File.mkdir_p(full) do
      {:reply, %{"status" => "ok"}, state}
    else
      {:error, reason} -> {:reply, %{"error" => format_error(reason)}, state}
    end
  end

  @impl Anthill.Plugin
  def handle_command(_cmd, _params, state) do
    {:reply, %{"error" => "unknown_command"}, state}
  end

  # ── GenServer callbacks ─────────────────────────────────────────

  @impl GenServer
  def handle_call({:command, cmd, params}, _from, state) do
    case handle_command(cmd, params, state) do
      {:reply, reply, new_state} -> {:reply, reply, new_state}
      {:noreply, new_state} -> {:reply, :ok, new_state}
    end
  end

  @impl GenServer
  def handle_cast({:command, cmd, params}, state) do
    case handle_command(cmd, params, state) do
      {:reply, _reply, new_state} -> {:noreply, new_state}
      {:noreply, new_state} -> {:noreply, new_state}
    end
  end

  # ── Path resolution and sandboxing ─────────────────────────────

  @doc """
  Resolve a relative path against the ANT root, rejecting escapes.

  Returns `{:ok, absolute_path}` or `{:error, reason}`.
  Rejects absolute paths and any `../` traversal that exits the sandbox.
  """
  @spec resolve(String.t(), String.t()) :: {:ok, String.t()} | {:error, String.t()}
  def resolve("/" <> _rest, _root), do: {:error, "absolute paths not allowed"}

  def resolve(path, root) do
    full = Path.expand(path, root)

    if String.starts_with?(full, root <> "/") or full == root do
      {:ok, full}
    else
      {:error, "access denied: path escapes sandbox"}
    end
  end

  defp format_error(reason) when is_atom(reason), do: Atom.to_string(reason)
  defp format_error(reason) when is_binary(reason), do: reason
  defp format_error(reason), do: inspect(reason)
end
