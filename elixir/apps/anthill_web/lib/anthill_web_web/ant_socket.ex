defmodule AnthillWebWeb.AntSocket do
  @moduledoc """
  Phoenix Socket for sentant communication.

  Clients connect via WebSocket at `/ws` and join a sentant channel
  with `sentant:<sentant_id>` topic.
  """

  use Phoenix.Socket

  channel "sentant:*", AnthillWebWeb.AntChannel
  channel "ant:*", AnthillWebWeb.AntChannel

  @impl Phoenix.Socket
  def connect(_params, socket, _connect_info) do
    {:ok, socket}
  end

  @impl Phoenix.Socket
  def id(_socket), do: nil
end
