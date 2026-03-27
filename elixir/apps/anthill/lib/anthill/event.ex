defmodule Anthill.Event do
  @moduledoc """
  R2 event structure for inter-sentant and intra-sentant communication.

  Events carry decisions (< 256 bytes). Plugins carry data.
  The `event_hash` is the FNV-1a hash of the event name, computed via NIF.
  """

  @type t :: %__MODULE__{
          name: String.t(),
          event_hash: non_neg_integer(),
          from: term(),
          to: term(),
          params: map()
        }

  @enforce_keys [:name, :event_hash]
  defstruct [
    :name,
    :event_hash,
    :from,
    :to,
    params: %{}
  ]

  @doc """
  Create an event from a name string.

  Computes the FNV-1a hash via the R2-FNV NIF. The `params` map holds
  event data (should be small — decisions, not bulk data).

  ## Options

    * `:from` — sender identifier (PID, ant_id, or `{:plugin, name}`)
    * `:to` — intended recipient (ant_id or nil for broadcast)

  ## Examples

      iex> Anthill.Event.new("#ping", %{"seq" => 1}, from: "alfred")
      %Anthill.Event{name: "#ping", event_hash: ..., from: "alfred", params: %{"seq" => 1}}
  """
  @spec new(String.t(), map(), keyword()) :: t()
  def new(name, params \\ %{}, opts \\ []) do
    {:ok, hash} = R2.Nif.r2_hash(name)

    %__MODULE__{
      name: name,
      event_hash: hash,
      from: Keyword.get(opts, :from),
      to: Keyword.get(opts, :to),
      params: params
    }
  end
end
