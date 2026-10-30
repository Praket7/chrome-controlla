// Presentation lease for a text transaction.
//
// A canvas editor is presented, not written to. When it stops being visible the
// transaction may reacquire the window's presentation slot, but it must never
// substitute DOM insertion for queued trusted input. This owns the token so the
// transaction only has to ask "can I still be seen?".

export function createPresentationLease(ctx) {
  let token = null;
  const acquire = async () => {
    if (token || typeof ctx.acquirePresentation !== "function") return token;
    token = await ctx.acquirePresentation().catch(() => null);
    return token;
  };
  return {
    get token() { return token; },
    acquire,
    async usable() {
      if (!(await acquire())) return false;
      if (typeof ctx.revalidatePresentation !== "function") return true;
      const valid = await ctx.revalidatePresentation(token).catch(() => false);
      if (!valid) token = null;
      return valid;
    },
    async release() {
      if (token && typeof ctx.releasePresentation === "function") {
        await ctx.releasePresentation(token).catch(() => {});
      }
      token = null;
    },
  };
}
