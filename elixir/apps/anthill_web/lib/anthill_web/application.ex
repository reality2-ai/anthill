defmodule AnthillWeb.Application do
  @moduledoc """
  AnthillWeb OTP application.

  Starts Phoenix endpoint and Absinthe subscription infrastructure.
  """

  use Application

  @impl Application
  def start(_type, _args) do
    children = [
      AnthillWebWeb.Telemetry,
      {DNSCluster, query: Application.get_env(:anthill_web, :dns_cluster_query) || :ignore},
      {Phoenix.PubSub, name: AnthillWeb.PubSub},
      AnthillWebWeb.Endpoint,
      {Absinthe.Subscription, AnthillWebWeb.Endpoint},
      AnthillWebWeb.EventBridge
    ]

    opts = [strategy: :one_for_one, name: AnthillWeb.Supervisor]
    Supervisor.start_link(children, opts)
  end

  @impl Application
  def config_change(changed, _new, removed) do
    AnthillWebWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
