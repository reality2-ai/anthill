defmodule AnthillWebWeb.ColonyController do
  @moduledoc """
  REST API for hive and sentant management.

  ## Endpoints

    * `GET /api/sentants` — list running sentants
    * `POST /api/sentants` — load a sentant or swarm from JSON definition
    * `DELETE /api/sentants/:id` — unload a sentant
    * `POST /api/sentants/:id/event` — send an event to a sentant
    * `GET /api/plugins` — list registered plugins
  """

  use AnthillWebWeb, :controller

  @doc "List all running sentants."
  def index(conn, _params) do
    sentants = R2.Hive.list_sentants()
    json(conn, %{sentants: sentants})
  end

  @doc """
  Load a sentant or swarm from a JSON definition.

  The body should be a valid R2-DEF JSON definition containing either
  a `sentant` or `swarm` key.
  """
  def create(conn, %{"sentant" => _} = definition) do
    load_and_respond(conn, definition)
  end

  def create(conn, %{"swarm" => _} = definition) do
    load_and_respond(conn, definition)
  end

  def create(conn, _params) do
    conn
    |> put_status(:bad_request)
    |> json(%{error: "body must contain 'sentant' or 'swarm' key"})
  end

  @doc "Unload a sentant by ID."
  def delete(conn, %{"id" => sentant_id}) do
    case R2.Hive.unload(sentant_id) do
      :ok ->
        json(conn, %{status: "unloaded", id: sentant_id})

      {:error, :not_found} ->
        conn |> put_status(:not_found) |> json(%{error: "not_found"})
    end
  end

  @doc "Send an event to a sentant."
  def send_event(conn, %{"id" => sentant_id} = params) do
    event = %{
      "event" => params["event"] || "",
      "parameters" => params["parameters"] || %{},
      "origin" => :external,
      "sender" => %{"sentant_id" => "web:" <> (params["sender"] || "anonymous")}
    }

    case R2.Hive.send_event(sentant_id, event) do
      :ok ->
        json(conn, %{status: "sent", id: sentant_id})

      {:error, :not_found} ->
        conn |> put_status(:not_found) |> json(%{error: "not_found"})
    end
  end

  @doc "List registered plugins."
  def plugins(conn, _params) do
    plugins = R2.Plugin.Manager.list_plugins()
    json(conn, %{plugins: plugins})
  end

  defp load_and_respond(conn, definition) do
    case R2.Hive.load_from_yaml(Jason.encode!(definition)) do
      {:ok, id} when is_binary(id) ->
        conn
        |> put_status(:created)
        |> json(%{status: "loaded", id: id})

      {:ok, ids} when is_list(ids) ->
        conn
        |> put_status(:created)
        |> json(%{status: "loaded", ids: ids, count: length(ids)})

      {:error, reason} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: inspect(reason)})
    end
  end
end
