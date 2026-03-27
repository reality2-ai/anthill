defmodule AnthillWebWeb.Schema.R2Subscriptions do
  @moduledoc "R2-GQL subscriptions (§6)."

  use Absinthe.Schema.Notation

  object :r2_subscriptions do
    @desc "Subscribe to events from a specific sentant."
    field :events, non_null(:hive_event) do
      arg :sentant_id, :id
      arg :name, :string

      config fn args, _ ->
        topic =
          case args do
            %{sentant_id: id, name: name} when is_binary(id) and is_binary(name) ->
              "events:#{id}:#{name}"

            %{sentant_id: id} when is_binary(id) ->
              "events:#{id}"

            _ ->
              "events:all"
          end

        {:ok, topic: topic}
      end
    end

    @desc "Subscribe to sentant lifecycle events (load/unload)."
    field :sentant_lifecycle, non_null(:sentant_lifecycle_event) do
      config fn _, _ -> {:ok, topic: "lifecycle"} end
    end
  end
end
