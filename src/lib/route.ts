import type { Server } from "../types";

export function jumpChainOf(server: Server): string[] {
  if (server.jumpChain?.length) return server.jumpChain;
  return server.jumpServerId ? [server.jumpServerId] : [];
}

export function jumpServersOf(server: Server, servers: Server[]): Server[] {
  return jumpChainOf(server)
    .map((id) => servers.find((item) => item.id === id))
    .filter((item): item is Server => Boolean(item));
}
