"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.LLMClient = void 0;
const ai_1 = require("ai");
const ai_sdk_provider_1 = require("@openrouter/ai-sdk-provider");
const rpc_js_1 = require("./rpc.js");
const openrouter = (0, ai_sdk_provider_1.createOpenRouter)({});
class LLMClient {
    async streamResponse(userMessage) {
        (0, rpc_js_1.emitRPC)({ type: 'turn_start' });
        try {
            const { textStream } = await (0, ai_1.streamText)({
                model: openrouter.chat('minimax/minimax-m2.5:free'),
                messages: [{ role: 'user', content: userMessage }],
            });
            for await (const chunk of textStream) {
                (0, rpc_js_1.emitRPC)({ type: 'text_delta', delta: chunk });
            }
        }
        catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            (0, rpc_js_1.emitRPC)({ type: 'error', message: `LLM error: ${message}` });
        }
        (0, rpc_js_1.emitRPC)({ type: 'turn_end' });
        (0, rpc_js_1.emitRPC)({ type: 'agent_end' });
    }
}
exports.LLMClient = LLMClient;
