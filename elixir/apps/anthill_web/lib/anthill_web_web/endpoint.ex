defmodule AnthillWebWeb.Endpoint do
  use Phoenix.Endpoint, otp_app: :anthill_web
  use Absinthe.Phoenix.Endpoint

  @session_options [
    store: :cookie,
    key: "_anthill_web_key",
    signing_salt: "x6RErvUq",
    same_site: "Lax"
  ]

  # Phoenix Channels (legacy ANT chat)
  socket "/ws", AnthillWebWeb.AntSocket,
    websocket: true

  # Absinthe GraphQL subscriptions
  socket "/gql-ws", AnthillWebWeb.GraphqlSocket,
    websocket: true,
    longpoll: false

  plug Plug.Static,
    at: "/",
    from: :anthill_web,
    gzip: not code_reloading?,
    only: AnthillWebWeb.static_paths(),
    raise_on_missing_only: code_reloading?

  if code_reloading? do
    plug Phoenix.CodeReloader
  end

  plug Plug.RequestId
  plug Plug.Telemetry, event_prefix: [:phoenix, :endpoint]

  plug Plug.Parsers,
    parsers: [:urlencoded, :multipart, :json],
    pass: ["*/*"],
    json_decoder: Phoenix.json_library()

  plug Plug.MethodOverride
  plug Plug.Head
  plug Plug.Session, @session_options
  plug AnthillWebWeb.Router
end
