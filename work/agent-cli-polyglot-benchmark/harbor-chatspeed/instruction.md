{
  "schema_version": "bowling_candidate_spec.v1",
  "candidate_mode": "chatspeed",
  "candidate_filename": "bowling.py",
  "runner_id": "harbor-python-unittest",
  "agent_id": "harbor-smoke-agent",
  "candidate_prompt": "Implement a ten-pin bowling scorer in the file /workspace/bowling.py (create that exact file; write nothing else into the workspace). The file must define a class BowlingGame with: __init__(self) starting a fresh game; roll(self, pins) recording one roll, raising ValueError when pins is outside 0-10 or when a frame's rolls would be invalid, and raising IndexError when the game is already complete; and score(self) returning the current total under standard ten-pin rules, including strike and spare bonuses and the tenth frame's bonus rolls. Use only the Python standard library. Do not modify any other file and do not use the network.",
  "artifact_policy": {"declared_only": true}
}
