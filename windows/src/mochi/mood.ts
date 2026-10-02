// Which of Mochi's existing emotes, if any, fits a chat exchange. Deliberately
// conservative: a wrong reaction on a technical answer looks worse than none, so
// it only speaks up when the text clearly calls for it. Nothing here leaves the
// machine — it is plain pattern matching on text the app already holds.

import type { BotEmoteName } from "../core/layout";

const THANKS = /\b(gracias|thanks?|thank you|merci|obrigad[oa]|danke|grazie)\b/i;
const APOLOGY = /\b(lo siento|perd[oó]n|disculp\w*|sorry|apolog\w*|d[ée]sol[ée]e?)\b/i;
/** Long answers are information, not a mood. */
const MAX_MOOD_LENGTH = 600;

export function moodOf(userText: string, replyText: string): BotEmoteName | null {
  if (THANKS.test(userText)) return "love";

  const reply = replyText.trim();
  if (!reply || reply.length > MAX_MOOD_LENGTH) return null;
  if (reply.includes("```") || reply.includes("`")) return null; // code: `!==`, `!important`…
  if (APOLOGY.test(reply)) return null; // never cheerful while apologising

  if (/!(\s|$)/.test(reply) || /¡/.test(reply)) return "happy";
  if (reply.endsWith("?") || reply.endsWith("¿")) return "surprised";
  return null;
}
