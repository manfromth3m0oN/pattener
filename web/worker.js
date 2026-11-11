// Runs the latexpat WebAssembly module off the main thread.
// Protocol: postMessage({id, request}) -> {id, response} (both JSON objects).
let exports = null;

async function load() {
  const { instance } = await WebAssembly.instantiateStreaming(fetch('latexpat.wasm'), {});
  exports = instance.exports;
}
const ready = load();

function call(request) {
  const input = new TextEncoder().encode(JSON.stringify(request));
  const ptr = exports.alloc(input.length);
  new Uint8Array(exports.memory.buffer, ptr, input.length).set(input);
  const out = exports.call(ptr, input.length);
  const len = new DataView(exports.memory.buffer).getUint32(out, true);
  const text = new TextDecoder().decode(new Uint8Array(exports.memory.buffer, out + 4, len));
  exports.dealloc(out, len + 4);
  return JSON.parse(text);
}

onmessage = async ({ data }) => {
  try {
    await ready;
    postMessage({ id: data.id, response: call(data.request) });
  } catch (e) {
    postMessage({ id: data.id, response: { ok: false, error: `internal error: ${e}` } });
  }
};
