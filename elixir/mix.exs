defmodule Anthill.Umbrella.MixProject do
  use Mix.Project

  def project do
    [
      apps_path: "apps",
      version: "0.1.0",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      listeners: [Phoenix.CodeReloader]
    ]
  end

  defp deps do
    [
      {:r2_nif, path: "../r2-core/elixir/apps/r2_nif", override: true},
      {:r2_core, path: "../r2-core/elixir/apps/r2_core", override: true}
    ]
  end
end
