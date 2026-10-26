# WRITEpad website

The marketing site for WRITEpad: home, download, pricing, and policies.
Built on the 2025 Hugo starter (Hugo 0.147, Tailwind + SCSS).

    npm install
    npm run dev      # http://localhost:1313
    npm run build    # -> public/

Netlify: set the base directory to `website/`; `netlify.toml` does the rest.

Settings live in `hugo.toml` under `[params]`: the version shown, the
GitHub links, the price, and `buyURL`, the checkout link for the $5 Publish
unlock. While `buyURL` is empty the buy buttons read "Coming soon".

## Contact form

`/contact/` is a Netlify Form named `contact`, but it posts to the
`/api/contact` function (`netlify/functions/contact.mts`) rather than straight
to Netlify. The function stops floods before anything is stored:

- Netlify rate limit: 5 posts a minute per IP (429 after that)
- daily caps in Netlify Blobs: 5 messages per IP, 100 for the whole site
- a honeypot field, a minimum time on the page, a size limit and a link limit

Then it forwards the message to Netlify Forms with a signature.
`netlify/functions/submission-created.mts` runs on every stored submission and
deletes contact messages without a valid signature, which means someone posted
to the form endpoint directly to get around the limits.

Netlify setup (Site configuration):

1. **Forms → Enable form detection**, then redeploy.
2. **Environment variables:**
   - `FORM_RELAY_SECRET`: any long random string (`openssl rand -hex 32`).
     Without it the form refuses to send.
   - `NETLIFY_API_TOKEN`: a personal access token (User settings →
     Applications) so unsigned submissions can be deleted. Without it they're
     only logged.
3. **Forms → Form notifications:** add an email notification for `contact`.

Limits are constants at the top of `contact.mts`. Free plans allow two
code-based rate-limit rules per project; this uses one.
