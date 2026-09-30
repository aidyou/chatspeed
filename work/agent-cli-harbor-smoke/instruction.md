{
  "schema_version": "campaign_plan.v1",
  "campaign_key": "harbor-smoke",
  "stage": "stage_0_manual",
  "agent_id": "harbor-smoke-agent",
  "suite": "chatspeed-smoke",
  "task": "smoke_reply_ok",
  "concurrency": 1,
  "budget": {
    "money_mode": { "mode": "token_resource_only" },
    "caps": { "input_tokens": 262144, "output_tokens": 16384 },
    "required_dimensions": [],
    "max_attempts": 1
  },
  "candidates": [
    { "candidate_key": "baseline", "kind": "baseline" },
    {
      "candidate_key": "cand-a",
      "kind": "candidate",
      "mutable_surface": ["agent_prompt_ref"],
      "agent_prompt_ref": "smoke-terse-v1",
      "prompt_hash": "bb41d700c9a2cdd26bffe26b5a3deac849188ce1966414c32700e38d51d2bf88"
    }
  ]
}
