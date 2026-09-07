export function withPathPrepended(
  env: NodeJS.ProcessEnv,
  dir?: string | null,
  platform: NodeJS.Platform = process.platform,
): NodeJS.ProcessEnv {
  const output = { ...env };

  if (platform !== "win32") {
    if (dir) {
      output.PATH = env.PATH ? `${dir}:${env.PATH}` : dir;
    }
    return output;
  }

  const inheritedKey = Object.keys(env).find((key) => key.toLowerCase() === "path");
  const inheritedValue = inheritedKey === undefined ? undefined : env[inheritedKey];

  for (const key of Object.keys(output)) {
    if (key.toLowerCase() === "path") delete output[key];
  }

  const pathValue = dir ? (inheritedValue ? `${dir};${inheritedValue}` : dir) : inheritedValue;
  if (pathValue !== undefined) {
    output[inheritedKey ?? "PATH"] = pathValue;
  }

  return output;
}
