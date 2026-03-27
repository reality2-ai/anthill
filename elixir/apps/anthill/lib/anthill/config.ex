defmodule Anthill.Config do
  @moduledoc """
  Load ANT configurations from TOML files.

  Directory structure:
    <colony_dir>/
    ├── supervisor.toml     — colony-level config
    └── ants/
        ├── alfred/
        │   ├── ant.toml    — ANT config
        │   └── working/    — sandboxed working directory
        └── boris/
            ├── ant.toml
            └── working/
  """

  require Logger

  @doc "Load colony config and return list of ANT configs."
  def load(colony_dir) do
    colony_dir = Path.expand(colony_dir)
    ants_dir = Path.join(colony_dir, "ants")

    unless File.dir?(ants_dir) do
      Logger.warning("No ants directory at #{ants_dir}")
      []
    else
      ants_dir
      |> File.ls!()
      |> Enum.filter(&File.dir?(Path.join(ants_dir, &1)))
      |> Enum.flat_map(fn ant_name ->
        ant_dir = Path.join(ants_dir, ant_name)
        config_path = Path.join(ant_dir, "ant.toml")

        if File.exists?(config_path) do
          case load_ant_config(ant_name, ant_dir, config_path) do
            {:ok, config} -> [config]
            {:error, reason} ->
              Logger.warning("Failed to load #{config_path}: #{reason}")
              []
          end
        else
          Logger.debug("No ant.toml in #{ant_dir}, skipping")
          []
        end
      end)
    end
  end

  defp load_ant_config(ant_name, ant_dir, config_path) do
    case File.read(config_path) do
      {:ok, content} ->
        case Toml.decode(content) do
          {:ok, toml} ->
            working_dir = Path.join(ant_dir, "working")
            File.mkdir_p!(working_dir)

            config = %{
              ant_id: ant_name,
              working_dir: working_dir,
              backend: get_in(toml, ["claude", "backend"]) || "claude-code",
              system_prompt: get_in(toml, ["claude", "system_prompt"]),
              timeout_ms: (get_in(toml, ["claude", "timeout_secs"]) || 120) * 1000
            }

            {:ok, config}

          {:error, reason} ->
            {:error, "TOML parse error: #{inspect(reason)}"}
        end

      {:error, reason} ->
        {:error, "File read error: #{reason}"}
    end
  end
end
