defmodule AnthillWebWeb.Schema.R2Types do
  @moduledoc """
  Core R2-GQL types from R2-GQL §3.
  """

  use Absinthe.Schema.Notation

  @desc "Arbitrary JSON scalar."
  scalar :json, name: "JSON" do
    serialize fn value -> value end
    parse fn
      %Absinthe.Blueprint.Input.String{value: value} -> Jason.decode(value)
      %Absinthe.Blueprint.Input.Null{} -> {:ok, nil}
      value -> {:ok, value}
    end
  end

  @desc "Raw YAML text."
  scalar :yaml, name: "YAML" do
    serialize fn value -> value end
    parse fn %Absinthe.Blueprint.Input.String{value: value} -> {:ok, value} end
  end

  @desc "A sentant — an autonomous FSM-based agent on a hive."
  object :sentant do
    @desc "UUID v4 assigned at load time."
    field :id, non_null(:id)

    @desc "Human-readable name from definition."
    field :name, non_null(:string)

    @desc "Reverse-DNS class string."
    field :class, :string

    @desc "Description from definition."
    field :description, non_null(:string)

    @desc "Public event declarations."
    field :public_events, non_null(list_of(non_null(:public_event)))

    @desc "Automation names (state/transition details are opaque)."
    field :automation_names, non_null(list_of(non_null(:string)))

    @desc "Bound plugin names."
    field :plugin_names, non_null(list_of(non_null(:string)))
  end

  @desc "A public event declared by a sentant."
  object :public_event do
    field :name, non_null(:string)
    field :description, :string
  end

  @desc "A swarm — a group of sentants loaded together."
  object :swarm do
    field :id, non_null(:id)
    field :name, non_null(:string)
    field :description, :string
    field :sentants, non_null(list_of(non_null(:sentant)))
  end

  @desc "Result of loading a sentant definition."
  object :sentant_load_result do
    field :success, non_null(:boolean)
    field :sentant, :sentant
    field :errors, non_null(list_of(non_null(:definition_error)))
  end

  @desc "Result of loading a swarm definition."
  object :swarm_load_result do
    field :success, non_null(:boolean)
    field :swarm, :swarm
    field :errors, non_null(list_of(non_null(:definition_error)))
  end

  @desc "Result of unloading."
  object :unload_result do
    field :success, non_null(:boolean)
    field :unloaded_ids, non_null(list_of(non_null(:id)))
    field :errors, non_null(list_of(non_null(:mutation_error)))
  end

  @desc "Result of sending an event."
  object :send_event_result do
    field :accepted, non_null(:boolean)
    field :dispatched_to, non_null(:integer)
    field :errors, non_null(list_of(non_null(:mutation_error)))
  end

  @desc "Definition validation error."
  object :definition_error do
    field :code, non_null(:string)
    field :message, non_null(:string)
  end

  @desc "Mutation error."
  object :mutation_error do
    field :code, non_null(:string)
    field :message, non_null(:string)
  end

  @desc "Event origin."
  enum :event_origin do
    value :internal
    value :external
    value :graphql
  end

  @desc "An event observed on the hive."
  object :hive_event do
    field :name, non_null(:string)
    field :origin, non_null(:event_origin)
    field :sentant_id, :id
    field :data, :json
    field :timestamp, non_null(:string)
  end

  @desc "Sentant lifecycle event."
  object :sentant_lifecycle_event do
    field :type, non_null(:string)
    field :sentant_id, non_null(:id)
    field :sentant_name, non_null(:string)
    field :sentant_class, :string
    field :timestamp, non_null(:string)
  end
end
