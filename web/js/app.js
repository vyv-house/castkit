/**
 * app.js — main application logic for screen-share web viewer
 */
import { VP8Decoder } from './decoder.js';
import { JitterBuffer } from './jitter-buffer.js';

const HEADER_SIZE = 9;
const TYPE_KEYFRAME = 0x01;
const TYPE_DELTA = 0x02;
const TYPE_META = 0x03;
const TYPE_FORCE_KEYFRAME = 0x04;

const connections = new Map(); // roomId -> connection state

export async function fetchRooms(baseUrl) {
    const res = await fetch(`http://${baseUrl}/api/rooms`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.json();
}

class RoomConnection {
    constructor(roomId, canvas, baseUrl) {
        this.roomId = roomId;
        this.canvas = canvas;
        this.baseUrl = baseUrl;
        this.decoder = new VP8Decoder(canvas);
        this.jitter = new JitterBuffer(50);
        this.ws = null;
        this.connected = false;
        this.reconnecting = false;
        this.reconnectDelay = 1000;
        this.maxReconnectDelay = 10000;
        this.reconnectTimer = null;
        this.rafId = null;
        this.lastWidth = 0;
        this.lastHeight = 0;
        this.frameCount = 0;
        this.lastFpsTime = 0;
        this.fps = 0;
        this.onStatusChange = null;
        this.onFpsUpdate = null;
        this._onMessage = this._onMessage.bind(this);
        this._onClose = this._onClose.bind(this);
        this._onError = this._onError.bind(this);
        this._onOpen = this._onOpen.bind(this);
        this._drain = this._drain.bind(this);
    }

    connect() {
        this._clearReconnect();
        this.reconnecting = true;
        this._notifyStatus();

        const url = `ws://${this.baseUrl}/watch/${this.roomId}`;
        try {
            this.ws = new WebSocket(url);
        } catch (e) {
            this._scheduleReconnect();
            return;
        }
        this.ws.binaryType = 'arraybuffer';
        this.ws.onopen = this._onOpen;
        this.ws.onmessage = this._onMessage;
        this.ws.onclose = this._onClose;
        this.ws.onerror = this._onError;
    }

    _onOpen() {
        this.connected = true;
        this.reconnecting = false;
        this.reconnectDelay = 1000;
        this._notifyStatus();
        this._startDrainLoop();
    }

    _onMessage(event) {
        const msg = event.data;
        if (typeof msg === 'string') {
            // text messages ignored for now
            return;
        }
        if (!(msg instanceof ArrayBuffer)) return;
        if (msg.byteLength < HEADER_SIZE) return;

        const view = new DataView(msg);
        const type = view.getUint8(0);
        const timestamp = view.getUint32(1, false); // big-endian
        const width = view.getUint16(5, false);
        const height = view.getUint16(7, false);
        const payload = new Uint8Array(msg, HEADER_SIZE);

        if (type === TYPE_META) {
            try {
                const text = new TextDecoder().decode(payload);
                const meta = JSON.parse(text);
                if (meta.width && meta.height) {
                    this._configure(meta.width, meta.height);
                }
            } catch (e) {
                // ignore malformed meta
            }
            return;
        }

        if (type === TYPE_FORCE_KEYFRAME) {
            if (this.ws && this.ws.readyState === WebSocket.OPEN) {
                this.ws.send('keyframe');
            }
            return;
        }

        if (width > 0 && height > 0 && (width !== this.lastWidth || height !== this.lastHeight)) {
            this._configure(width, height);
        }

        this.jitter.push({ type, timestamp, data: payload });
    }

    _configure(width, height) {
        this.lastWidth = width;
        this.lastHeight = height;
        this.canvas.width = width;
        this.canvas.height = height;
        this.decoder.configure(width, height).catch((e) => {
            console.warn('Decoder configure failed:', e);
        });
        if (this.onStatusChange) this.onStatusChange(this._getStatus());
    }

    _drain() {
        this.rafId = requestAnimationFrame(this._drain);
        const nowMs = performance.now();
        const frames = this.jitter.drain(nowMs);
        for (const f of frames) {
            this.decoder.decode(f.type, f.timestamp, f.data);
            this.frameCount++;
        }
        if (nowMs - this.lastFpsTime >= 1000) {
            this.fps = this.frameCount;
            this.frameCount = 0;
            this.lastFpsTime = nowMs;
            if (this.onFpsUpdate) this.onFpsUpdate(this.fps);
        }
        if (this.decoder.requestKeyframe()) {
            if (this.ws && this.ws.readyState === WebSocket.OPEN) {
                this.ws.send('keyframe');
            }
        }
    }

    _startDrainLoop() {
        if (this.rafId) return;
        this.lastFpsTime = performance.now();
        this.frameCount = 0;
        this.rafId = requestAnimationFrame(this._drain);
    }

    _stopDrainLoop() {
        if (this.rafId) {
            cancelAnimationFrame(this.rafId);
            this.rafId = null;
        }
    }

    _onClose() {
        this.connected = false;
        this._notifyStatus();
        this._stopDrainLoop();
        this._scheduleReconnect();
    }

    _onError(err) {
        console.warn('WebSocket error for room', this.roomId, err);
        this.connected = false;
        this._notifyStatus();
    }

    _scheduleReconnect() {
        this._clearReconnect();
        this.reconnecting = true;
        this._notifyStatus();
        this.reconnectTimer = setTimeout(() => {
            this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxReconnectDelay);
            this.connect();
        }, this.reconnectDelay);
    }

    _clearReconnect() {
        if (this.reconnectTimer) {
            clearTimeout(this.reconnectTimer);
            this.reconnectTimer = null;
        }
    }

    _getStatus() {
        if (this.connected) return 'connected';
        if (this.reconnecting) return 'reconnecting';
        return 'disconnected';
    }

    _notifyStatus() {
        if (this.onStatusChange) this.onStatusChange(this._getStatus());
    }

    disconnect() {
        this._clearReconnect();
        this._stopDrainLoop();
        if (this.ws) {
            try {
                this.ws.close();
            } catch (e) {
                // ignore
            }
            this.ws = null;
        }
        this.decoder.destroy();
        this.jitter.clear();
        this.connected = false;
        this.reconnecting = false;
        this._notifyStatus();
    }
}

export function connectRoom(roomId, canvas, baseUrl) {
    if (connections.has(roomId)) {
        connections.get(roomId).disconnect();
    }
    const conn = new RoomConnection(roomId, canvas, baseUrl);
    connections.set(roomId, conn);
    conn.connect();
    return conn;
}

export function disconnectRoom(roomId) {
    const conn = connections.get(roomId);
    if (conn) {
        conn.disconnect();
        connections.delete(roomId);
    }
}

export function getConnection(roomId) {
    return connections.get(roomId) || null;
}
