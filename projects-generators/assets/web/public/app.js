const result = document.querySelector("#result");
const form = document.querySelector("form");
const mode = document.body.dataset.mode;

async function api(path, method = "GET", body) {
  const response = await fetch(new URL(window.API_BASE + path, window.location.href), {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`API returned ${response.status}`);
  return response.json();
}

async function submit(event) {
  event.preventDefault();
  const button = form.querySelector("button");
  button.disabled = true;
  try {
    const message = form.elements.message.value;
    if (mode === "jobs") {
      const job = await api("jobs", "POST", { message });
      result.textContent = `Queued ${job.id}`;
      for (let attempt = 0; attempt < 60; attempt += 1) {
        await new Promise((resolve) => setTimeout(resolve, 500));
        const current = await api(`jobs/${job.id}`);
        if (current.status === "done") {
          result.textContent = current.result;
          return;
        }
      }
      throw new Error(`Job ${job.id} is still queued; inspect the worker`);
    }
    result.textContent = (await api("greeting", "PUT", { message })).message;
  } catch (error) {
    result.textContent = error.message;
  } finally {
    button.disabled = false;
  }
}

form.addEventListener("submit", submit);
if (mode === "jobs") {
  result.textContent = "Ready to send a greeting.";
} else {
  api("greeting").then((greeting) => {
    result.textContent = greeting.message;
  }).catch((error) => { result.textContent = error.message; });
}
