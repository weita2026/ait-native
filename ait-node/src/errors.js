export class NativeBridgeError extends Error {
  constructor(message, options) {
    super(message, options);
    this.name = new.target.name;
  }
}

export class NativeResolutionError extends NativeBridgeError {}

export class NativeProtocolError extends NativeBridgeError {}
