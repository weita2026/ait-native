export const LANGUAGE_BINDING_CONTRACT = "ait.language.binding.v1";
export const AGENT_CAPABILITIES_CONTRACT =
  "ait.agent.worker.capabilities.v1";
export const SUPPORTED_TRANSPORTS = Object.freeze([
  "telegram",
  "discord",
  "slack",
  "line",
]);
export const SUPPORTED_WORKER_OPERATIONS = Object.freeze([
  "slack-command",
  "discord-interaction",
  "reply-provider",
]);
