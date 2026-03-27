defmodule AnthillWebWeb.Schema do
  @moduledoc """
  Root GraphQL schema.

  Core R2-GQL types and operations, plus Anthill extensions.
  """

  use Absinthe.Schema

  import_types AnthillWebWeb.Schema.R2Types
  import_types AnthillWebWeb.Schema.R2Queries
  import_types AnthillWebWeb.Schema.R2Mutations
  import_types AnthillWebWeb.Schema.R2Subscriptions
  import_types AnthillWebWeb.Schema.AnthillMutations

  query do
    import_fields :r2_queries
  end

  mutation do
    import_fields :r2_mutations
    import_fields :anthill_mutations
  end

  subscription do
    import_fields :r2_subscriptions
  end
end
