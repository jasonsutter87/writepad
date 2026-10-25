// Sends the contact form without leaving the page, and shows errors from a
// no-JavaScript post (/contact/?error=...). The form works without this file.
(function () {
  var form = document.getElementById('form');
  var status = document.getElementById('form-status');
  if (!form) return;

  var messages = {
    sent: 'Thanks, your message is on its way. You’ll hear back by email.',
    invalid: 'Please add your email address and a message.',
    links: 'That message has a lot of links in it. Please trim them and try again.',
    busy: 'Too many messages right now. Please try again later, or open an issue on GitHub.',
    failed: 'Something went wrong sending that. Please try again in a minute.'
  };

  function show(code) {
    status.textContent = messages[code] || messages.failed;
    status.className = 'form-status' + (code === 'sent' ? ' is-ok' : ' is-error');
    status.hidden = false;
  }

  form.elements.started.value = String(Date.now());

  var error = new URLSearchParams(location.search).get('error');
  if (error) show(error);

  form.addEventListener('submit', function (e) {
    e.preventDefault();
    var button = form.querySelector('button[type="submit"]');
    button.disabled = true;
    fetch(form.action, { method: 'POST', body: new URLSearchParams(new FormData(form)), headers: { Accept: 'application/json' } })
      .then(function (res) {
        if (res.status === 429 && !res.headers.get('content-type')?.includes('json')) return { code: 'busy' };
        return res.json();
      })
      .then(function (r) {
        show(r.code);
        if (r.code === 'sent') form.reset();
      })
      .catch(function () { show('failed'); })
      .finally(function () {
        button.disabled = false;
        form.elements.started.value = String(Date.now());
      });
  });
})();
