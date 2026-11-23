# Automatic model selection

> **Historical document (2026-10-28).** Server, Auto/funding, hosted relay and
> native-agent claims below describe an earlier design, not the current app.
> The inference server is removed; chat/routines use installed CLI engines,
> Groq/OpenRouter/fal operations use the user's keys directly, and sharing has
> no replacement. Current boundaries are in
> [ADR-0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md),
> [llm-providers.md](llm-providers.md), and [configuration.md](configuration.md).


Clovy supports `open-software/auto` while retaining explicit model selection. Auto persists a
cost-to-quality preference (`cost_quality`, 0 to 1) and forwards it through the Clovy-owned agent
runtime and note generation.

Clovy API resolves Auto itself, per request, to a concrete priced OpenRouter model. Nothing named
`open-software/auto` is sent upstream:

| `cost_quality` | Model |
| - | - |
| below 0.34 | `nvidia/nemotron-3-nano-30b-a3b` |
| 0.34 to 0.66 | `moonshotai/kimi-k2.6` |
| above 0.66, or absent | `z-ai/glm-5.2` |

When an agent request needs image input or tools and the tier model lacks that capability (from
the OpenRouter catalog's input modalities and supported parameters), Clovy uses
`moonshotai/kimi-k2.6`, then any priced model that has the capability, else it fails
`model_capability_unavailable`. An unpriced legacy text id resolves through Auto. The tier models
live in the API's Auto resolver; changing them is a code change reviewed against the catalog.

The catalog only says some endpoint of a model supports a parameter; OpenRouter treats `tools` and
`response_format` as a soft routing preference and silently ignores them on an endpoint that lacks
them. So every agent request that sends `tools`, `tool_choice` or `response_format` also sends
`provider.require_parameters: true`: the endpoint that serves it must support every parameter in
the request, or OpenRouter does not route it at all rather than dropping the tools or the output
schema. A plain request keeps OpenRouter's default routing.

Auto is Clovy-funded: a user's own OpenRouter key is stripped from an explicit Auto request, and a
stale model id with a user key fails `openrouter_api_key_model_unavailable` instead of silently
moving to Clovy credits. Billing uses the resolved concrete model's catalog price, and every
Clovy-metered request asks OpenRouter for routes no more expensive than that price
(`provider.max_price`), so the upstream cost never exceeds the charge.

OpenRouter routes requests to third-party providers under its own and each provider's data
policies. Clovy makes no confidential-compute or zero-retention claim for Auto or any other
OpenRouter model, and sends no routing-privacy header.

For an agent run, Auto selects a concrete model on the first inference. Tool continuations and
approval resumes pin that selected model for the remainder of the agent run so provider-native
reasoning history remains compatible. Clovy also preserves the reasoning field observed on that
response when replaying assistant history, rather than deriving the provider wire format from a
model ID. A later user-initiated agent run evaluates Auto again.

Build the desktop release with `OS_CLOVY_AUTO_MODE_DEFAULT=true`. Existing users retain their saved
model. Roll back by removing the build flag.
