// Runs after Netlify Forms stores a submission. Contact messages that didn't
// come through the contact function (no valid relay signature) were posted
// to the form endpoint directly, around the rate limits: delete them.
//
// Needs NETLIFY_API_TOKEN (a personal access token) to delete; without it,
// unsigned submissions are only logged.

import { verify } from "../shared/relay.ts";

type Event = { body: string | null };

export const handler = async (event: Event) => {
  const { payload } = JSON.parse(event.body ?? "{}");
  if (payload?.form_name !== "contact") return { statusCode: 200 };

  const secret = process.env.FORM_RELAY_SECRET;
  const data = payload.data ?? {};
  if (secret && verify(secret, data.relay, String(data.message ?? ""))) return { statusCode: 200 };

  const token = process.env.NETLIFY_API_TOKEN;
  if (!token) {
    console.warn(`Unsigned contact submission ${payload.id}; set NETLIFY_API_TOKEN to delete these.`);
    return { statusCode: 200 };
  }
  const res = await fetch(`https://api.netlify.com/api/v1/submissions/${payload.id}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${token}` },
  });
  console.log(`Deleted unsigned contact submission ${payload.id}: ${res.status}`);
  return { statusCode: 200 };
};
