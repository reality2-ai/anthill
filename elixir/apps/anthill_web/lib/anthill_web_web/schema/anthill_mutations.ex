defmodule AnthillWebWeb.Schema.AnthillMutations do
  @moduledoc """
  Anthill-specific GraphQL mutations (extensions to R2-GQL).

  These provide convenience operations for ANT management that
  aren't part of the core R2-GQL spec.
  """

  use Absinthe.Schema.Notation

  object :anthill_mutations do
    @desc "Create a default ANT with AI and knowledge plugins."
    field :create_ant, non_null(:sentant_load_result) do
      @desc "ANT name."
      arg :name, non_null(:string)
      @desc "Optional description."
      arg :description, :string
      @desc "AI backend (default: claude-code)."
      arg :backend, :string

      resolve fn _, %{name: name} = args, _ ->
        definition = Anthill.Definitions.ant(name,
          description: args[:description] || "An AI assistant ANT"
        )

        case R2.Hive.load_from_yaml(Jason.encode!(definition)) do
          {:ok, id} ->
            {:ok, %{
              success: true,
              sentant: R2.Hive.get_sentant(id),
              errors: []
            }}

          {:error, reason} ->
            {:ok, %{
              success: false,
              sentant: nil,
              errors: [%{code: "LOAD_FAILED", message: inspect(reason)}]
            }}
        end
      end
    end
  end
end
