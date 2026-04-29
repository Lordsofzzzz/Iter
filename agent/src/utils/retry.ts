import { logToFile } from "./logger";
import { emitEvent } from "../rpc.js";

function abortableSleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(new DOMException("Aborted", "AbortError")); return; }
    const t = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => {
      clearTimeout(t);
      reject(new DOMException("Aborted", "AbortError"));
    }, { once: true });
  });
}

export async function retry<T>(
  fn: () => Promise<T>,
  signal: AbortSignal,
  retries = 5,
  delay = 2000,
  attempt = 1,
): Promise<T> {
  try {
    const result = await fn();
    if (attempt > 1) {
      emitEvent({ type: 'retry_result', success: true, attempt });
    }
    return result;
  } catch (err: any) {
    const isAbort = err?.name === "AbortError" || signal.aborted;
    if (isAbort) throw err;

    const is429 =
      err?.statusCode === 429 ||
      err?.code === 429 ||
      err?.message?.includes("rate");

    if (!is429 || retries === 0) {
      if (attempt > 1) {
        emitEvent({ type: 'retry_result', success: false, attempt });
      }
      logToFile(`Error: ${err?.message ?? JSON.stringify(err)}`);
      throw err;
    }

    logToFile(`Retrying in ${delay}ms (${retries} left)`);
    emitEvent({ type: 'cooldown', wait_ms: delay, retries_left: retries });

    try {
      await abortableSleep(delay, signal);
    } catch {
      throw new DOMException("Aborted", "AbortError");
    }

    emitEvent({ type: 'turn_start' });
    return retry(fn, signal, retries - 1, delay * 2, attempt + 1);
  }
}