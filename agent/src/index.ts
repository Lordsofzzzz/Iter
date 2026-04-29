import { LLMClient, MODEL_NAME, MODEL_LIMIT, MODEL_TEMP, clearHistory } from './llm/index.js';
import { emitEvent, emitResponse, readStdinLines, SessionStatsData } from './rpc.js';

const llm = new LLMClient();
let isStreaming = false;

emitEvent({ type: 'agent_start' });

readStdinLines(async (line) => {
  let payload: any;
  try {
    payload = JSON.parse(line);
  } catch {
    emitEvent({ type: 'error', message: 'Invalid JSON on stdin' });
    return;
  }

  const id: string | undefined = typeof payload.id === 'string' ? payload.id : undefined;

  switch (payload.type) {

    case 'get_state':
      emitResponse({
        kind: 'response', command: 'get_state', id, success: true,
        data: {
          model_name:   MODEL_NAME,
          model_limit:  MODEL_LIMIT,
          temp:         MODEL_TEMP,
          is_streaming: isStreaming,
        },
      });
      break;

    case 'get_session_stats': {
      const data = llm.getSessionStatsResponse(MODEL_LIMIT);
      emitResponse({ kind: 'response', command: 'get_session_stats', id, success: true, data });
      break;
    }

    case 'abort':
      llm.abort();
      emitResponse({ kind: 'response', command: 'abort', id, success: true });
      break;

    case 'clear':
      clearHistory(llm);
      emitResponse({ kind: 'response', command: 'clear', id, success: true });
      break;

    case 'prompt':
    case 'message': {
      if (typeof payload.content !== 'string') {
        emitResponse({ kind: 'response', command: 'prompt', id, success: false, error: 'content must be a string' });
        break;
      }
      if (isStreaming) {
        emitResponse({ kind: 'response', command: 'prompt', id, success: false, error: 'Agent busy' });
        break;
      }
      emitResponse({ kind: 'response', command: 'prompt', id, success: true });
      isStreaming = true;
      await llm.streamResponse(payload.content);
      isStreaming = false;
      break;
    }

    default:
      emitResponse({
        kind: 'response', command: payload.type ?? 'unknown', id,
        success: false, error: `Unknown command: ${payload.type}`,
      });
  }
});