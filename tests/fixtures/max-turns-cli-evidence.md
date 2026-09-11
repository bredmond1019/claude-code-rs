# `--max-turns` CLI fixture evidence

This is **real, captured** evidence that the installed `claude` CLI accepts `--max-turns` in
print mode. `claude --help` does not document the flag (see the block record's `what` field), so
this manual, once-only, billed invocation is the only proof it is honored. Nothing here is
hand-written to match the parser — see `README.md` in this directory for the provenance
convention this fixtures directory follows.

## `claude --version`

```
2.1.268 (Claude Code)
```

## Invocation

```
claude -p --max-turns 1 "Reply with exactly the single word: pong" --output-format json
```

## Exit code

```
0
```

## stdout (raw JSON envelope, `session_id` and `uuid` redacted to the all-zero sentinel per this
directory's convention)

```json
{"duration_api_ms":2174,"stop_reason":"end_turn","session_id":"00000000-0000-0000-0000-000000000000","total_cost_usd":0.4517555,"usage":{"input_tokens":2,"cache_creation_input_tokens":44562,"cache_read_input_tokens":10139,"output_tokens":4,"output_tokens_details":{"thinking_tokens":0},"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":44562,"ephemeral_5m_input_tokens":0},"inference_geo":"not_available","iterations":[{"input_tokens":2,"output_tokens":4,"cache_read_input_tokens":10139,"cache_creation_input_tokens":44562,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":44562},"type":"message"}],"speed":"standard"},"modelUsage":{"claude-haiku-4-5-20251001":{"inputTokens":901,"outputTokens":11,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.000956,"contextWindow":200000,"maxOutputTokens":32000,"thinkingTokens":0,"canonicalModel":"claude-haiku-4-5","provider":"firstParty","costBasis":"list"},"claude-opus-5":{"inputTokens":2,"outputTokens":4,"cacheReadInputTokens":10139,"cacheCreationInputTokens":44562,"webSearchRequests":0,"costUSD":0.45079949999999996,"contextWindow":1000000,"maxOutputTokens":64000,"thinkingTokens":0,"canonicalModel":"claude-opus-5","provider":"firstParty","costBasis":"list"}},"permission_denials":[],"terminal_reason":"completed","fast_mode_state":"off","fast_mode_disabled_reason":"sdk_opt_in_required","subagent_stats":{"spawned":0,"requested":{"background":0,"foreground":0,"unset":0},"started_in_background":0,"max_depth":0,"spawned_by_subagents":0,"completed":0,"failed":0,"killed":{"parent":0,"user":0,"system":0},"refused":{"depth_limit":0,"concurrency_limit":0,"budget":0},"by_type":{}},"is_error":false,"num_turns":1,"subtype":"success","api_error_status":null,"result":"pong","ttft_ms":1330,"type":"result","duration_ms":1348,"uuid":"00000000-0000-0000-0000-000000000000","ttft_stream_ms":1213,"time_to_request_ms":19,"first_content_frame_ms":1213,"queued_turn_count":0,"result_index":0}
```

## Interpretation

The call exited `0` with `is_error: false`, `num_turns: 1`, `terminal_reason: "completed"`, and
`result: "pong"` — the exact single-word reply asked for. Nothing in the output looks like an
error or an unrecognized-flag rejection (no `is_error: true`, no non-zero exit, no CLI usage
message on stderr — stderr carried only an unrelated `SessionEnd` hook failure). This is a
single-turn prompt bounded by a turn ceiling of 1, so the call is expected to complete normally
regardless of whether `--max-turns` is enforced or silently ignored — this invocation therefore
cannot distinguish "honored" from "silently accepted", but it does establish that the flag is
**accepted** (no rejection) and does not break a bounded single-turn call, which is consistent
with (not proof beyond doubt of) `--max-turns` being honored.
