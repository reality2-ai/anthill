defmodule Anthill.Plugin do
  @moduledoc """
  Behaviour for ANT plugins.

  Plugins perform I/O, manage external resources, and interact with the
  world. Sentants (Conductors) are pure FSMs — they decide what to do.
  Plugins do it.

  Each plugin is a GenServer. Each ANT gets its own instance of each
  plugin it declares — no shared singletons.
  """

  @type command_reply :: {:reply, map(), term()}
  @type command_noreply :: {:noreply, term()}
  @type command_result :: command_reply() | command_noreply()
  @type event_result :: {:ok, term()}

  @doc "Handle a command from the Conductor. Returns a reply map or :noreply."
  @callback handle_command(command :: String.t(), params :: map(), state :: term()) ::
              command_result()

  @doc "Handle an event arriving at this plugin (async)."
  @callback handle_event(event :: Anthill.Event.t(), state :: term()) ::
              event_result()

  @doc "Periodic tick for self-driven work (retry, decay, maintenance)."
  @callback handle_tick(state :: term()) ::
              event_result()

  @optional_callbacks handle_event: 2, handle_tick: 1
end
