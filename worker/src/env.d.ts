import { Env as WorkerEnv } from './index';

declare module 'cloudflare:test' {
  interface ProvidedEnv extends WorkerEnv {}
}

declare global {
  namespace Cloudflare {
    interface Env extends WorkerEnv {}
  }
}
