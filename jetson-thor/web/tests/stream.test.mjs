import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chatEvents } from '../stream.js';

const encode = (text) => new TextEncoder().encode(text);
const collect = async (body, signal = new AbortController().signal, idleMs) => {
  const events = [];
  for await (const event of chatEvents(body, signal, idleMs)) events.push(event);
  return events;
};

test('DONE completes and cancels a body whose server keeps the connection open', async () => {
  let cancelled = false;
  const body = new ReadableStream({
    start(controller) { controller.enqueue(encode('data: {"choices":[]}\n\ndata: [DONE]\n\n')); },
    cancel() { cancelled = true; },
  });

  assert.deepEqual(await collect(body), [{ choices: [] }]);
  assert.equal(cancelled, true);
  assert.equal(body.locked, false);
});

test('UTF-8, multiline events and CRLF survive arbitrary byte boundaries', async () => {
  const bytes = encode(': heartbeat\r\ndata: {"text":\r\ndata: "雪🐯"}\r\n\r\ndata: [DONE]\r\n\r\n');
  const body = new ReadableStream({
    start(controller) {
      for (const byte of bytes) controller.enqueue(new Uint8Array([byte]));
      controller.close();
    },
  });

  assert.deepEqual(await collect(body), [{ text: '雪🐯' }]);
});

test('Stop interrupts a pending read and releases the stream', async () => {
  const controller = new AbortController();
  let cancelled = false;
  const body = new ReadableStream({ cancel() { cancelled = true; } });
  const result = collect(body, controller.signal);

  controller.abort();

  await assert.rejects(result, { name: 'AbortError' });
  assert.equal(cancelled, true);
  assert.equal(body.locked, false);
});

test('silent connections time out and get cancelled', async () => {
  let cancelled = false;
  const body = new ReadableStream({ cancel() { cancelled = true; } });

  await assert.rejects(collect(body, undefined, 10), /stopped sending data/);
  assert.equal(cancelled, true);
});

test('truncated and malformed streams report an error', async () => {
  const body = (text) => new ReadableStream({ start(controller) {
    controller.enqueue(encode(text));
    controller.close();
  } });

  await assert.rejects(collect(body('data: {"choices":[]}\n\n')), /before completion/);
  await assert.rejects(collect(body('data: invalid\n\n')), SyntaxError);
});
