declare const __APP_COMMIT_HASH__: string;

interface ImportMetaEnv {
  readonly VITE_CLOVY_REPLAY_ONBOARDING?: string;
}

declare module "*.svg" {
  const src: string;
  export default src;
}
