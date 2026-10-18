import { test } from 'node:test';
import assert from 'node:assert/strict';
import { LocalHistory, newConversation } from '../history.js';

class MemoryStorage {
  data = new Map();
  get length() { return this.data.size; }
  key(index) { return [...this.data.keys()][index]; }
  getItem(key) { return this.data.get(key) ?? null; }
  setItem(key, value) { this.data.set(key, value); }
  removeItem(key) { this.data.delete(key); }
}

const chat = (content) => ({ ...newConversation(), messages: [{ role: 'user', content }] });

test('migrates the old chat without losing it and restores it after reload', () => {
  const storage = new MemoryStorage();
  storage.setItem('messages', JSON.stringify(chat('Existing conversation').messages));
  const history = new LocalHistory(storage);
  const restored = history.restore();

  assert.equal(restored.messages[0].content, 'Existing conversation');
  assert.equal(storage.getItem('messages'), null);
  assert.equal(new LocalHistory(storage).restore().id, restored.id);
});

test('keeps multiple chats and drafts; deletion leaves other chats intact', () => {
  const history = new LocalHistory(new MemoryStorage());
  const first = chat('First');
  const second = { ...chat('Second'), draft: 'Unsent\n\ndraft' };

  history.save(first);
  history.save(second);

  assert.equal(history.list().length, 2);
  assert.equal(history.restore().draft, second.draft);
  history.remove(first.id);

  assert.deepEqual(history.list().map((item) => item.id), [second.id]);
});

test('independent tabs do not overwrite unrelated conversations', () => {
  const storage = new MemoryStorage();
  const one = new LocalHistory(storage);
  const two = new LocalHistory(storage);

  one.save(chat('One'));
  two.save(chat('Two'));

  assert.equal(one.list().length, 2);
});

test('quota failure keeps legacy history and reports that saving failed', () => {
  const storage = new MemoryStorage();
  storage.setItem('messages', JSON.stringify(chat('Do not lose me').messages));
  storage.setItem = () => { throw new Error('Quota exceeded'); };
  const history = new LocalHistory(storage);

  assert.equal(history.restore().messages[0].content, 'Do not lose me');
  assert.ok(storage.getItem('messages'));
  assert.match(history.error, /could not be saved/);
});

test('malformed entries cannot break the rest of the history', () => {
  const storage = new MemoryStorage();
  storage.setItem('thor.chat.v1.broken', '{');
  const history = new LocalHistory(storage);
  const good = chat('Still readable');

  history.save(good);

  assert.equal(history.list().length, 1);
  assert.equal(history.restore().id, good.id);
});

test('partial replies restore as interrupted, without a permanent streaming state', () => {
  const history = new LocalHistory(new MemoryStorage());
  const value = chat('Explain');
  value.messages.push({ role: 'assistant', content: 'Partial', streaming: true });
  history.save(value);
  const reply = history.restore().messages[1];

  assert.equal(reply.streaming, undefined);
  assert.match(reply.stats, /interrupted/);
});
