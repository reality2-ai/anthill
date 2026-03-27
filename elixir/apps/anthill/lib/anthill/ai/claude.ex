defmodule Anthill.AI.Claude do
  @moduledoc """
  Claude Code backend.

  Spawns `claude` as a managed subprocess (Elixir Port) with its working
  directory locked to the ANT's sandbox. Uses --print for single-shot
  prompts.
  """

  @behaviour Anthill.AI.Backend

  @impl true
  def name, do: "claude-code"

  @impl true
  def available? do
    System.find_executable("claude") != nil
  end

  @impl true
  def execute(request) do
    claude = System.find_executable("claude")

    unless claude do
      {:error, "claude binary not found in PATH"}
    else
      args = build_args(request)
      start = System.monotonic_time(:millisecond)

      # Use sh -c to redirect stdin from /dev/null (prevents Claude's stdin warning)
      sh = System.find_executable("sh")
      cmd = Enum.join([claude | args] |> Enum.map(&shell_escape/1), " ")

      port =
        Port.open({:spawn_executable, sh}, [
          :binary,
          :exit_status,
          :stderr_to_stdout,
          {:cd, request.working_dir},
          {:args, ["-c", cmd <> " </dev/null"]}
        ])

      collect_output(port, [], request.timeout_ms, start)
    end
  end

  defp build_args(request) do
    args = ["--print"]

    args =
      if request.system_prompt do
        args ++ ["--system-prompt", request.system_prompt]
      else
        args
      end

    args ++ [request.prompt]
  end

  defp shell_escape(arg) do
    "'" <> String.replace(arg, "'", "'\\''") <> "'"
  end

  defp collect_output(port, acc, timeout_ms, start) do
    receive do
      {^port, {:data, data}} ->
        collect_output(port, [data | acc], timeout_ms, start)

      {^port, {:exit_status, code}} ->
        duration = System.monotonic_time(:millisecond) - start
        output = acc |> Enum.reverse() |> IO.iodata_to_binary()

        if code == 0 do
          {:ok, %{output: output, exit_code: code, duration_ms: duration}}
        else
          {:error, "claude exited with code #{code}: #{String.slice(output, 0, 500)}"}
        end
    after
      timeout_ms ->
        Port.close(port)
        {:error, "timeout after #{timeout_ms}ms"}
    end
  end
end
