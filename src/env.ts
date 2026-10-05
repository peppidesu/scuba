import { defineEnvVars } from "@sveltejs/kit/env";

export const variables = defineEnvVars({
  DATABASE_URL: { description: "The database connection string." },
  ORIGIN: {
    description: "The app origin (base URL), e.g. `http://localhost:5173`.",
  },
  BETTER_AUTH_SECRET: {
    description:
      "Secret used to sign tokens. For production use 32 characters generated with high entropy. See [Better Auth installation](https://www.better-auth.com/docs/installation).",
  },
  DISCORD_CLIENT_ID: {
    description: "Discord OAuth client ID.",
  },
  DISCORD_CLIENT_SECRET: {
    description: "Discord OAuth client secret.",
  },
});
