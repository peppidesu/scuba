import { fail, redirect } from "@sveltejs/kit";
import type { Actions } from "./$types";
import type { PageServerLoad } from "./$types";
import { auth } from "#lib/server/auth.ts";

export const load: PageServerLoad = (event) => {
  if (event.locals.user) {
    return redirect(302, "/demo/better-auth");
  }
  return {};
};

export const actions: Actions = {
  signInSocial: async (event) => {
    const formData = await event.request.formData();
    const provider = formData.get("provider")?.toString() ?? "discord";
    const callbackURL = formData.get("callbackURL")?.toString() ?? "/demo/better-auth";

    const result = await auth.api.signInSocial({
      body: {
        provider: provider as "discord",
        callbackURL,
      },
    });

    if (result.url) {
      return redirect(302, result.url, { external: true });
    }
    return fail(400, { message: "Social sign-in failed" });
  },
};
