defmodule AnthillWebWeb.Schema.R2Queries do
  @moduledoc "R2-GQL queries (§4)."

  use Absinthe.Schema.Notation

  object :r2_queries do
    @desc "Get a sentant by ID."
    field :sentant, :sentant do
      arg :id, non_null(:id)
      resolve &AnthillWebWeb.Resolvers.Sentant.get/3
    end

    @desc "List sentants on the local hive."
    field :sentants, non_null(list_of(non_null(:sentant))) do
      arg :class, :string
      arg :name_contains, :string
      resolve &AnthillWebWeb.Resolvers.Sentant.list/3
    end

    @desc "List registered plugins."
    field :plugins, non_null(list_of(non_null(:string))) do
      resolve fn _, _, _ -> {:ok, R2.Plugin.Manager.list_plugins()} end
    end
  end
end
