const PASSWORD = 'infrarust-e2e-password';

export class Yggdrasil {
  constructor(baseUrl) {
    this.baseUrl = baseUrl.replace(/\/$/, '');
    this.cache = new Map();
  }

  get hasJoinedUrl() {
    return `${this.baseUrl}/session/minecraft/hasJoined`;
  }

  get sessionServer() {
    return this.baseUrl;
  }

  async ready({ timeoutMs = 20000 } = {}) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      try {
        const res = await fetch(`${this.baseUrl}/authlib-injector`);
        if (res.ok) return;
      } catch {
      }
      if (Date.now() > deadline) throw new Error(`no Yggdrasil server answering at ${this.baseUrl}`);
      await new Promise((r) => setTimeout(r, 200));
    }
  }

  async session(username) {
    const cached = this.cache.get(username);
    if (cached) return cached;

    await this.createUser(username);

    const res = await fetch(`${this.baseUrl}/authenticate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        username,
        password: PASSWORD,
        agent: { name: 'Minecraft', version: 1 },
        requestUser: true,
      }),
    });
    if (!res.ok) throw new Error(`authenticate failed for ${username}: ${res.status} ${await res.text()}`);
    const body = await res.json();

    const session = {
      username: body.selectedProfile.name,
      uuid: body.selectedProfile.id,
      accessToken: body.accessToken,
    };
    this.cache.set(username, session);
    return session;
  }

  async createUser(username) {
    const res = await fetch(`${this.baseUrl}/drasl/api/v2/users`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username, password: PASSWORD, requestApiToken: true }),
    });
    if (res.ok) return;
    const text = await res.text();
    if (/exists|taken|duplicate/i.test(text)) return;
    throw new Error(`could not create ${username}: ${res.status} ${text}`);
  }
}
