const nonce = location.hash.slice(1);
const form = document.querySelector("#form");
const input = document.querySelector("#password");
const context = document.querySelector("#context");
const status = document.querySelector("#status");
let finished = false;

async function send(message) {
  return chrome.runtime.sendMessage({ ...message, nonce });
}

async function initialize() {
  if (!/^[a-f0-9]{64}$/.test(nonce)) throw new Error("This one-use request is invalid or expired.");
  const result = await send({ type: "secure-input-ready" });
  if (!result?.ok) throw new Error("This one-use request is invalid or expired.");
  context.textContent = `Site: ${result.origin}\nField: ${result.label}`;
  input.focus();
}

form.addEventListener("submit", async event => {
  event.preventDefault();
  if (finished || !input.value) return;
  finished = true;
  let password = input.value;
  input.value = "";
  input.disabled = true;
  status.textContent = "Sending once to the selected password field…";
  try {
    const result = await send({ type: "secure-input-submit", password });
    if (!result?.ok) throw new Error("The request expired. Return to the page and take a fresh snapshot.");
    status.textContent = "Password sent once. This tab will close.";
  } catch (error) {
    status.textContent = error.message || "The request failed.";
    finished = false;
    input.disabled = false;
  } finally {
    password = "";
  }
});

document.querySelector("#cancel").addEventListener("click", async () => {
  if (finished) return;
  finished = true;
  input.value = "";
  await send({ type: "secure-input-cancel" }).catch(() => {});
  window.close();
});

void initialize().catch(error => {
  form.hidden = true;
  status.textContent = error.message || "This request is invalid or expired.";
});
