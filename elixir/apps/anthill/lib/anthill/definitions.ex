defmodule Anthill.Definitions do
  @moduledoc """
  Default sentant definitions for Anthill ANTs.

  These are R2-DEF compliant definitions that use the
  ai.reality2.ai and ai.reality2.knowledge plugins.
  """

  @doc """
  Default ANT definition — AI assistant with persistent knowledge.

  Flow: prompt → knowledge context → AI with context → store learning → respond
  """
  @spec ant(String.t(), keyword()) :: map()
  def ant(name, opts \\ []) do
    description = Keyword.get(opts, :description, "An AI assistant ANT")
    class = Keyword.get(opts, :class, "ai.reality2.ant.assistant")

    %{
      "sentant" => %{
        "name" => name,
        "description" => description,
        "class" => class,
        "plugins" => [
          %{"name" => "ai.reality2.ai"},
          %{"name" => "ai.reality2.knowledge"}
        ],
        "automations" => [
          %{
            "name" => "main",
            "transitions" => [
              # Boot
              %{"from" => "start", "event" => "init", "to" => "idle"},

              # User sends prompt → save sender, retrieve knowledge context
              %{
                "from" => "idle",
                "event" => "prompt",
                "to" => "retrieving",
                "public" => true,
                "actions" => [
                  %{"command" => "set", "parameters" => %{
                    "key" => "user_prompt",
                    "value" => "{{params.text}}"
                  }},
                  %{"command" => "set", "parameters" => %{
                    "key" => "reply_to",
                    "value" => "{{params.sender.sentant_id}}"
                  }},
                  %{"plugin" => "ai.reality2.knowledge", "command" => "query",
                    "parameters" => %{"about" => "{{params.text}}"}}
                ]
              },

              # Knowledge context arrives → send to AI with context
              %{
                "from" => "retrieving",
                "event" => "ai.reality2.knowledge.query",
                "to" => "thinking",
                "actions" => [
                  %{"command" => "set", "parameters" => %{
                    "key" => "context",
                    "value" => "{{params.data.context}}"
                  }},
                  %{"plugin" => "ai.reality2.ai", "command" => "query",
                    "parameters" => %{
                      "prompt" => "{{vars.user_prompt}}",
                      "system_prompt" => "You are #{name}. Use this context from your memory:\n{{vars.context}}"
                    }}
                ]
              },

              # AI responds → store in knowledge → reply to original sender
              %{
                "from" => "thinking",
                "event" => "ai.reality2.ai.query",
                "to" => "idle",
                "actions" => [
                  %{"command" => "set", "parameters" => %{
                    "key" => "response",
                    "value" => "{{params.data.output}}"
                  }},
                  %{"plugin" => "ai.reality2.knowledge", "command" => "update",
                    "parameters" => %{
                      "input" => "{{vars.user_prompt}}",
                      "output" => "{{vars.response}}"
                    }},
                  %{"command" => "send", "parameters" => %{
                    "event" => "response",
                    "to" => "{{vars.reply_to}}",
                    "output" => "{{vars.response}}",
                    "backend" => "{{params.data.backend}}"
                  }}
                ]
              },

              # Ping/pong (any state)
              %{"from" => "*", "event" => "ping", "public" => true,
                "actions" => [
                  %{"command" => "send", "parameters" => %{
                    "event" => "pong", "to" => "@sender"
                  }}
                ]},

              # Knowledge stats request (any state)
              %{"from" => "*", "event" => "knowledge_stats", "public" => true,
                "actions" => [
                  %{"command" => "set", "parameters" => %{
                    "key" => "stats_reply_to",
                    "value" => "{{params.sender.sentant_id}}"
                  }},
                  %{"plugin" => "ai.reality2.knowledge", "command" => "stats",
                    "parameters" => %{}}
                ]},

              # Knowledge stats result → forward to requester
              %{"from" => "*", "event" => "ai.reality2.knowledge.stats",
                "actions" => [
                  %{"command" => "send", "parameters" => %{
                    "event" => "knowledge_stats_result",
                    "to" => "{{vars.stats_reply_to}}",
                    "node_count" => "{{params.data.node_count}}",
                    "edge_count" => "{{params.data.edge_count}}",
                    "avg_confidence" => "{{params.data.avg_confidence}}"
                  }}
                ]},

              # Knowledge update result (consumed silently — no action needed)
              %{"from" => "*", "event" => "ai.reality2.knowledge.update"}
            ]
          }
        ]
      }
    }
  end
end
