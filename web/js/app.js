import { MonitorGrid } from './grid.js';

export class StreamManager {
    static MonitorGrid = MonitorGrid;

    constructor(grid, baseUrl, options = {}) {
        this.grid = grid;
        this.baseUrl = baseUrl;
        this.maxStreams = options.maxStreams || 12;
        this.streams = new Map();
        this.elements = new Map();
        this.pinnedRooms = new Set();
        this.autoAddInterval = null;
    }

    addStream(roomId, options = {}) {
        if (options.pinned) {
            this.pinnedRooms.add(roomId);
        }
        if (this.streams.has(roomId)) {
            return this.streams.get(roomId);
        }
        if (this.streams.size >= this.maxStreams) {
            console.warn(`Stream limit reached; not adding ${roomId}`);
            return null;
        }

        const { canvas, element } = this.grid.addMonitor(roomId);
        this.elements.set(roomId, element);
        this.grid.setStatus(roomId, 'disconnected');

        if (!canvas.transferControlToOffscreen || typeof Worker === 'undefined') {
            this.streams.set(roomId, null);
            this.grid.setStatus(roomId, 'disconnected');
            this.grid.setError(roomId, 'Unsupported browser');
            console.warn('This browser does not support worker-based canvas rendering');
            return null;
        }

        let offscreen;
        let worker;
        try {
            offscreen = canvas.transferControlToOffscreen();
            worker = new Worker(new URL('./stream-worker.js', import.meta.url));
        } catch (error) {
            this.streams.set(roomId, null);
            this.grid.setStatus(roomId, 'disconnected');
            this.grid.setError(roomId, 'Worker failed');
            console.warn(`Failed to start stream worker for ${roomId}:`, error);
            return null;
        }

        worker.addEventListener('message', (event) => {
            const message = event.data;

            if (message.type === 'status') {
                const status = message.connected
                    ? 'connected'
                    : message.reconnecting
                        ? 'reconnecting'
                        : 'disconnected';
                this.grid.setStatus(roomId, status);
            } else if (message.type === 'resolution') {
                this.grid.setResolution(roomId, message.width, message.height);
            } else if (message.type === 'metrics') {
                if (Number.isFinite(message.fps)) {
                    this.grid.setFps(roomId, message.fps);
                }
                this.grid.setMetrics(roomId, message);
            } else if (message.type === 'fatal') {
                this.grid.setStatus(roomId, 'disconnected');
                this.grid.setError(roomId, message.message || 'Unsupported browser');
                console.warn(`Stream worker fatal error for ${roomId}:`, message.message);
            } else if (message.type === 'error') {
                console.warn(`Stream worker error for ${roomId}:`, message.message);
            }
        });

        worker.addEventListener('error', (error) => {
            this.grid.setStatus(roomId, 'disconnected');
            this.grid.setError(roomId, error.message || 'Worker failed');
            console.warn(`Stream worker failed for ${roomId}:`, error.message || error);
        });

        worker.postMessage({
            type: 'start',
            canvas: offscreen,
            wsUrl: this.createWatchUrl(roomId),
        }, [offscreen]);

        this.streams.set(roomId, worker);
        return worker;
    }

    createWatchUrl(roomId) {
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        return `${protocol}//${this.baseUrl}/watch/${encodeURIComponent(roomId)}`;
    }

    removeStream(roomId) {
        if (!this.streams.has(roomId)) {
            return;
        }

        const worker = this.streams.get(roomId);
        if (worker) {
            worker.terminate();
        }
        this.grid.removeMonitor(roomId);
        this.elements.delete(roomId);
        this.streams.delete(roomId);
        this.pinnedRooms.delete(roomId);
    }

    removeAll() {
        for (const roomId of Array.from(this.streams.keys())) {
            this.removeStream(roomId);
        }
    }

    startAutoAdd() {
        if (this.autoAddInterval) {
            return;
        }

        let syncInFlight = false;
        const syncRooms = async () => {
            if (syncInFlight) {
                return;
            }

            syncInFlight = true;
            try {
                const rooms = await fetchRooms(this.baseUrl);
                const roomIds = new Set(rooms.map((room) => room.id).filter(Boolean));

                for (const roomId of roomIds) {
                    if (!this.streams.has(roomId)) {
                        try {
                            this.addStream(roomId);
                        } catch (error) {
                            console.warn(`Failed to add stream ${roomId}:`, error);
                        }
                    }
                }

                for (const roomId of Array.from(this.streams.keys())) {
                    if (!roomIds.has(roomId) && !this.pinnedRooms.has(roomId)) {
                        this.removeStream(roomId);
                    }
                }
            } catch (error) {
                console.warn('Failed to fetch rooms:', error);
            } finally {
                syncInFlight = false;
            }
        };

        syncRooms();
        this.autoAddInterval = setInterval(syncRooms, 3000);
    }

    stopAutoAdd() {
        if (!this.autoAddInterval) {
            return;
        }

        clearInterval(this.autoAddInterval);
        this.autoAddInterval = null;
    }
}

export async function fetchRooms(baseUrl) {
    void baseUrl;
    const res = await fetch(`/api/rooms`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.json();
}
