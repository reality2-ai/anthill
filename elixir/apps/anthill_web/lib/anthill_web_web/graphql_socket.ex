defmodule AnthillWebWeb.GraphqlSocket do
  @moduledoc """
  Phoenix Socket for GraphQL subscriptions via Absinthe.

  Clients connect via WebSocket at `/gql-ws` and use the
  Absinthe subscription protocol.
  """

  use Phoenix.Socket

  use Absinthe.Phoenix.Socket,
    schema: AnthillWebWeb.Schema

  @impl Phoenix.Socket
  def connect(_params, socket, _connect_info) do
    {:ok, socket}
  end

  @impl Phoenix.Socket
  def id(_socket), do: nil
end
