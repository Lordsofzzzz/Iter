#!/bin/bash
export OPENROUTER_API_KEY=""
export MODEL_NAME="minimax/minimax-m2.5-20260211:free"
cd agent
echo '{"type":"prompt","content":"hi"}' | timeout 15 bun run src/index.ts