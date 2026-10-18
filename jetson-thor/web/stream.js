// Read SSE events independently of network chunk boundaries. Closing the
// reader also closes the fetch body, releasing the upstream generation slot.
export async function* chatEvents(body, signal, idleMs = 90000) {
  const reader = body.getReader();
  const decoder = new TextDecoder();

  let buffer = "";
  let data = [];
  let failure;
  let timer;

  const cancel = () => { reader.cancel().catch(() => {}); };
  const aborted = () => {
    failure = signal.reason || new DOMException("Stopped", "AbortError");
    cancel();
  };

  signal.addEventListener("abort", aborted, { once: true });

  try {
    for (;;) {
      if (signal.aborted) aborted();
      if (failure) throw failure;

      timer = setTimeout(() => {
        failure = new Error("The server stopped sending data. Try again.");
        cancel();
      }, idleMs);

      const { value, done } = await reader.read();
      clearTimeout(timer);

      if (failure) throw failure;

      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true });
      if (done && buffer) buffer += "\n\n";

      let end;

      while ((end = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, end).replace(/\r$/, "");
        buffer = buffer.slice(end + 1);

        if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));

        if (line === "" && data.length) {
          const payload = data.join("\n");
          data = [];

          if (payload.trim() === "[DONE]") return;

          yield JSON.parse(payload);
        }
      }

      if (done) throw new Error("The response ended before completion. The partial reply was saved.");
    }
  } finally {
    clearTimeout(timer);
    signal.removeEventListener("abort", aborted);

    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
