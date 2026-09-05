import { Effect } from "effect";

import server from "../index.js";

const id = "aft-opencode";

// Do not start the server when this module is imported;
// start it only when the host invokes the server initializer.
const effect = () => Effect.succeed(undefined);

export default {
  id,
  server,
  effect,
};
