defmodule AnthillWebWeb.EventBridge do
  @moduledoc """
  Bridges R2 hive events to GraphQL subscriptions.

  Registers on the R2 EventBus and publishes matching events to
  Absinthe subscription topics. This is the "ephemeral entanglement"
  between the hive's event stream and connected WebSocket clients
  (R2-GQL §6).
  """

  use GenServer
  require Logger

  @doc "Start the event bridge."
  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl GenServer
  def init(_opts) do
    Registry.register(R2.EventBus, :events, [])
    Logger.info("[event_bridge] Bridging R2 events to GraphQL subscriptions")
    {:ok, %{}}
  end

  @impl GenServer
  def handle_info({:event, event}, state) when is_map(event) do
    publish_event(event)
    {:noreply, state}
  end

  def handle_info(_, state), do: {:noreply, state}

  defp publish_event(event) do
    event_name = Map.get(event, "event", "")
    params = Map.get(event, "parameters", %{})
    origin = Map.get(event, "origin", :internal)
    sender = Map.get(event, "sender", %{})
    sentant_id = Map.get(sender, "sentant_id", "")
    timestamp = DateTime.utc_now() |> DateTime.to_iso8601()

    hive_event = %{
      name: event_name,
      origin: to_origin_enum(origin),
      sentant_id: sentant_id,
      data: params,
      timestamp: timestamp
    }

    # Publish to all matching subscription topics
    Absinthe.Subscription.publish(
      AnthillWebWeb.Endpoint,
      hive_event,
      events: "events:all"
    )

    if sentant_id != "" do
      Absinthe.Subscription.publish(
        AnthillWebWeb.Endpoint,
        hive_event,
        events: "events:#{sentant_id}"
      )

      Absinthe.Subscription.publish(
        AnthillWebWeb.Endpoint,
        hive_event,
        events: "events:#{sentant_id}:#{event_name}"
      )
    end

    # Lifecycle events
    if event_name in ["sentant.loaded", "sentant.unloaded"] do
      lifecycle_event = %{
        type: if(event_name == "sentant.loaded", do: "LOADED", else: "UNLOADED"),
        sentant_id: params["sentant_id"] || sentant_id,
        sentant_name: params["sentant_name"] || "",
        sentant_class: params["sentant_class"] || "",
        timestamp: timestamp
      }

      Absinthe.Subscription.publish(
        AnthillWebWeb.Endpoint,
        lifecycle_event,
        sentant_lifecycle: "lifecycle"
      )
    end
  end

  defp to_origin_enum(:internal), do: :internal
  defp to_origin_enum(:external), do: :external
  defp to_origin_enum(:graphql), do: :graphql
  defp to_origin_enum(_), do: :internal
end
