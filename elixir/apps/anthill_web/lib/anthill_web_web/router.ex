defmodule AnthillWebWeb.Router do
  use AnthillWebWeb, :router

  pipeline :api do
    plug :accepts, ["json"]
  end

  # GraphQL endpoint (R2-GQL)
  scope "/" do
    pipe_through :api

    forward "/graphql", Absinthe.Plug,
      schema: AnthillWebWeb.Schema,
      json_codec: Jason

    # GraphiQL development interface
    forward "/graphiql", Absinthe.Plug.GraphiQL,
      schema: AnthillWebWeb.Schema,
      socket: AnthillWebWeb.AntSocket,
      json_codec: Jason,
      interface: :playground
  end

  # Serve the UI at root
  scope "/", AnthillWebWeb do
    get "/", PageController, :index
  end

  # REST API (kept for backward compatibility and simple clients)
  scope "/api", AnthillWebWeb do
    pipe_through :api

    get "/sentants", ColonyController, :index
    post "/sentants", ColonyController, :create
    delete "/sentants/:id", ColonyController, :delete
    post "/sentants/:id/event", ColonyController, :send_event
    get "/plugins", ColonyController, :plugins
  end
end
