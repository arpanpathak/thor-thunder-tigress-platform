// Each conversation has its own key, so saving one tab cannot overwrite
// unrelated conversations in another. No history is uploaded or synced.
const PREFIX = "thor.chat.v1.";
const ACTIVE = "thor.chat.active";

export const preferences = {
  get(key, fallback) {
    try {
      const value = localStorage.getItem(key);

      return value === null ? fallback : JSON.parse(value);
    } catch {
      return fallback;
    }
  },

  set(key, value) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      // Preferences are optional; conversation storage reports failures below.
    }
  },
};

function validMessages(messages) {
  return Array.isArray(messages) && messages.every((message) =>
    message && ["user", "assistant"].includes(message.role) && typeof message.content === "string");
}

export function newConversation() {
  return { id: crypto.randomUUID(), updatedAt: Date.now(), messages: [], draft: "" };
}

export function conversationTitle(conversation) {
  const first = conversation.messages.find((message) => message.role === "user");

  return (first?.content || conversation.draft || "New chat").replace(/\s+/g, " ").trim().slice(0, 70);
}

export class LocalHistory {
  constructor(storage) {
    this.error = "";

    try {
      this.storage = storage ?? globalThis.localStorage;
    } catch {
      this.error = "Local storage is unavailable. Keep this tab open or export your chat.";
    }
  }

  read(id) {
    try {
      const value = JSON.parse(this.storage.getItem(PREFIX + id));

      if (value?.id !== id || !validMessages(value.messages)) return null;

      return {
        ...value,
        draft: typeof value.draft === "string" ? value.draft : "",
        messages: value.messages.map(({ streaming, ...message }) => message),
      };
    } catch {
      return null;
    }
  }

  list() {
    try {
      return Array.from({ length: this.storage.length }, (_, index) => this.storage.key(index))
        .filter((key) => key?.startsWith(PREFIX))
        .map((key) => this.read(key.slice(PREFIX.length)))
        .filter(Boolean)
        .sort((a, b) => b.updatedAt - a.updatedAt);
    } catch {
      this.error = "Local storage is unavailable. Keep this tab open or export your chat.";
      return [];
    }
  }

  restore() {
    try {
      const active = this.read(this.storage.getItem(ACTIVE));
      if (active) return active;

      const previous = this.list()[0];
      if (previous) return previous;

      const legacy = JSON.parse(this.storage.getItem("messages"));

      if (validMessages(legacy) && legacy.length) {
        const migrated = { ...newConversation(), messages: legacy };
        if (this.save(migrated)) this.storage.removeItem("messages");

        return migrated;
      }
    } catch {
      this.error = "Local history could not be read. Existing browser data has been left untouched.";
    }

    return newConversation();
  }

  save(conversation) {
    try {
      const messages = conversation.messages.map(({ streaming, ...message }) => ({
        ...message,
        ...(streaming ? { stats: "Reply interrupted — saved locally" } : {}),
      }));

      this.storage.setItem(PREFIX + conversation.id, JSON.stringify({ ...conversation, messages }));
      this.storage.setItem(ACTIVE, conversation.id);
      this.error = "";

      return true;
    } catch {
      this.error = "This chat could not be saved locally. Browser storage may be full or blocked. Export it before closing.";
      return false;
    }
  }

  remove(id) {
    try {
      this.storage.removeItem(PREFIX + id);
      this.error = "";

      return true;
    } catch {
      this.error = "This chat could not be deleted from browser storage.";

      return false;
    }
  }
}
