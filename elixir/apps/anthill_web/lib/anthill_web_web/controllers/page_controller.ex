defmodule AnthillWebWeb.PageController do
  use AnthillWebWeb, :controller

  @doc "Serve the Anthill UI."
  def index(conn, _params) do
    path = Application.app_dir(:anthill_web, "priv/static/index.html")

    conn
    |> put_resp_content_type("text/html")
    |> send_file(200, path)
  end
end
