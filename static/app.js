const elements = {
  conversation: document.querySelector("#conversation"),
  sender: document.querySelector("#sender"),
  join: document.querySelector("#join"),
  messages: document.querySelector("#messages"),
  empty: document.querySelector("#empty"),
  composer: document.querySelector("#composer"),
  body: document.querySelector("#body"),
  send: document.querySelector("#send"),
  install: document.querySelector("#install"),
  notifications: document.querySelector("#notifications"),
  status: document.querySelector("#status"),
  statusDot: document.querySelector("#status-dot"),
};

const profileId = localStorage.getItem("cassy-profile-id") || crypto.randomUUID();
localStorage.setItem("cassy-profile-id", profileId);
elements.sender.value = localStorage.getItem("cassy-sender") || elements.sender.value;
const requestedConversation = new URLSearchParams(location.search).get("conversation");
elements.conversation.value = requestedConversation || localStorage.getItem("cassy-conversation") || "demo";

let socket;
let connectionGeneration = 0;
let displayedMessages = new Set();
let installPrompt;

function currentConversation() {
  return elements.conversation.value.trim() || "demo";
}

function currentSender() {
  return elements.sender.value.trim() || "anonymous";
}

async function registerProfile() {
  const response = await fetch("/api/profiles", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ profile_id: profileId, display_name: currentSender() }),
  });
  if (!response.ok) throw new Error(`Profile request failed: ${response.status}`);
}

function setConnectionStatus(text, online = false) {
  elements.status.textContent = text;
  elements.statusDot.classList.toggle("online", online);
}

function formatTime(timestamp) {
  const milliseconds = timestamp < 10_000_000_000 ? timestamp * 1000 : timestamp;
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(milliseconds));
}

function renderMessage(message) {
  if (displayedMessages.has(message.message_id)) return;
  displayedMessages.add(message.message_id);
  elements.empty.hidden = true;

  const article = document.createElement("article");
  article.className = "message";
  if (message.sender_id === profileId) article.classList.add("mine");

  const meta = document.createElement("div");
  meta.className = "message-meta";

  const sender = document.createElement("span");
  sender.textContent = message.sender_name || message.sender_id;
  const time = document.createElement("time");
  time.textContent = formatTime(message.sent_at);
  meta.append(sender, time);

  const body = document.createElement("p");
  body.className = "message-body";
  body.textContent = message.body;

  article.append(meta, body);
  elements.messages.append(article);
  elements.messages.scrollTop = elements.messages.scrollHeight;
}

async function loadHistory(conversation) {
  const response = await fetch(`/api/conversations/${encodeURIComponent(conversation)}/messages`);
  if (!response.ok) throw new Error(`History request failed: ${response.status}`);
  const history = await response.json();
  history.reverse().forEach(renderMessage);
}

function connect(conversation) {
  connectionGeneration += 1;
  const generation = connectionGeneration;
  if (socket) socket.close();

  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${protocol}//${location.host}/ws/${encodeURIComponent(conversation)}`);
  setConnectionStatus("Connecting");

  socket.addEventListener("open", () => setConnectionStatus("Live", true));
  socket.addEventListener("message", (event) => renderMessage(JSON.parse(event.data)));
  socket.addEventListener("close", () => {
    if (generation !== connectionGeneration) return;
    setConnectionStatus("Reconnecting");
    window.setTimeout(() => connect(conversation), 1500);
  });
  socket.addEventListener("error", () => socket.close());
}

async function joinConversation() {
  const conversation = currentConversation();
  localStorage.setItem("cassy-conversation", conversation);
  localStorage.setItem("cassy-sender", currentSender());

  displayedMessages = new Set();
  elements.messages.replaceChildren(elements.empty);
  elements.empty.hidden = false;
  elements.join.disabled = true;

  try {
    await registerProfile();
    await loadHistory(conversation);
    connect(conversation);
    elements.body.focus();
  } catch (error) {
    setConnectionStatus("Unavailable");
    console.error(error);
  } finally {
    elements.join.disabled = false;
  }
}

elements.join.addEventListener("click", joinConversation);

elements.composer.addEventListener("submit", async (event) => {
  event.preventDefault();
  const body = elements.body.value.trim();
  if (!body) return;

  elements.send.disabled = true;
  try {
    const response = await fetch("/api/messages", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        conversation_id: currentConversation(),
        sender_id: profileId,
        body,
      }),
    });
    if (!response.ok) throw new Error(`Send failed: ${response.status}`);
    elements.body.value = "";
    elements.body.focus();
  } catch (error) {
    setConnectionStatus("Send failed");
    console.error(error);
  } finally {
    elements.send.disabled = false;
  }
});

function urlBase64ToBytes(value) {
  const padding = "=".repeat((4 - (value.length % 4)) % 4);
  const decoded = atob((value + padding).replaceAll("-", "+").replaceAll("_", "/"));
  return Uint8Array.from(decoded, (character) => character.charCodeAt(0));
}

async function enableNotifications() {
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) {
    throw new Error("Push notifications are not supported by this browser");
  }

  const keyResponse = await fetch("/api/push/vapid-public-key");
  if (!keyResponse.ok) throw new Error("Web Push is not configured on the server");
  const { public_key: publicKey } = await keyResponse.json();
  const registration = await navigator.serviceWorker.ready;
  let subscription = await registration.pushManager.getSubscription();
  if (!subscription) {
    subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToBytes(publicKey),
    });
  }

  const serialized = subscription.toJSON();
  const response = await fetch("/api/push/subscriptions", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      user_id: profileId,
      conversation_id: currentConversation(),
      endpoint: serialized.endpoint,
      keys: serialized.keys,
    }),
  });
  if (!response.ok) throw new Error(`Subscription request failed: ${response.status}`);
  elements.notifications.textContent = "Notifications on";
}

elements.notifications.addEventListener("click", async () => {
  elements.notifications.disabled = true;
  try {
    await registerProfile();
    await enableNotifications();
  } catch (error) {
    const denied = "Notification" in window && Notification.permission === "denied";
    elements.notifications.textContent = denied ? "Notifications blocked" : "Try notifications";
    console.error(error);
  } finally {
    elements.notifications.disabled = false;
  }
});

window.addEventListener("beforeinstallprompt", (event) => {
  event.preventDefault();
  installPrompt = event;
  elements.install.hidden = false;
});

elements.install.addEventListener("click", async () => {
  if (!installPrompt) return;
  await installPrompt.prompt();
  installPrompt = undefined;
  elements.install.hidden = true;
});

window.addEventListener("appinstalled", () => {
  installPrompt = undefined;
  elements.install.hidden = true;
});

async function start() {
  if ("serviceWorker" in navigator) {
    await navigator.serviceWorker.register("/service-worker.js");
    if ("PushManager" in window) {
      const subscription = await (await navigator.serviceWorker.ready).pushManager.getSubscription();
      if (subscription) elements.notifications.textContent = "Notifications on";
    } else {
      elements.notifications.hidden = true;
    }
  } else {
    elements.notifications.hidden = true;
  }
  await joinConversation();
}

start().catch((error) => {
  setConnectionStatus("Unavailable");
  console.error(error);
});
