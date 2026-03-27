defmodule AnthillWebWeb.Schema.R2Mutations do
  @moduledoc "R2-GQL mutations (§5)."

  use Absinthe.Schema.Notation

  object :r2_mutations do
    @desc "Load a sentant from a JSON/YAML definition."
    field :sentant_load, non_null(:sentant_load_result) do
      arg :definition, non_null(:string)
      resolve &AnthillWebWeb.Resolvers.Sentant.load/3
    end

    @desc "Unload a sentant by ID."
    field :sentant_unload, non_null(:unload_result) do
      arg :id, non_null(:id)
      resolve &AnthillWebWeb.Resolvers.Sentant.unload/3
    end

    @desc "Send an event to a sentant via the management plane."
    field :send_event, non_null(:send_event_result) do
      @desc "Target sentant ID."
      arg :sentant_id, non_null(:id)
      @desc "Event name."
      arg :event, non_null(:string)
      @desc "Event data payload."
      arg :data, :json
      resolve &AnthillWebWeb.Resolvers.Sentant.send_event/3
    end
  end
end
