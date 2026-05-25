/**
 * GridManager — multi-view grid layout manager
 */
export class GridManager {
    constructor(container) {
        this.container = container;
        this.rooms = new Map(); // roomId -> { cell, canvas, label, statusDot }
        this._boundResize = () => this.updateLayout();
        window.addEventListener('resize', this._boundResize);
    }

    addRoom(roomId) {
        if (this.rooms.has(roomId)) {
            return this.rooms.get(roomId).canvas;
        }

        const cell = document.createElement('div');
        cell.className = 'grid-cell';
        cell.dataset.roomId = roomId;

        const canvasWrap = document.createElement('div');
        canvasWrap.className = 'canvas-wrap';

        const canvas = document.createElement('canvas');
        canvas.className = 'room-canvas';
        canvasWrap.appendChild(canvas);

        const overlay = document.createElement('div');
        overlay.className = 'cell-overlay';

        const label = document.createElement('span');
        label.className = 'cell-label';
        label.textContent = roomId;

        const statusDot = document.createElement('span');
        statusDot.className = 'status-dot disconnected';
        statusDot.title = 'Disconnected';

        overlay.appendChild(statusDot);
        overlay.appendChild(label);
        cell.appendChild(canvasWrap);
        cell.appendChild(overlay);
        this.container.appendChild(cell);

        const entry = { cell, canvas, label, statusDot };
        this.rooms.set(roomId, entry);
        this.updateLayout();
        return canvas;
    }

    removeRoom(roomId) {
        const entry = this.rooms.get(roomId);
        if (!entry) return;
        entry.cell.remove();
        this.rooms.delete(roomId);
        this.updateLayout();
    }

    getCanvas(roomId) {
        const entry = this.rooms.get(roomId);
        return entry ? entry.canvas : null;
    }

    setStatus(roomId, status) {
        const entry = this.rooms.get(roomId);
        if (!entry) return;
        entry.statusDot.className = 'status-dot ' + status;
        entry.statusDot.title =
            status === 'connected'
                ? 'Connected'
                : status === 'reconnecting'
                ? 'Reconnecting'
                : 'Disconnected';
    }

    updateLayout() {
        const count = this.rooms.size;
        if (count === 0) {
            this.container.style.gridTemplateColumns = '1fr';
            return;
        }
        const width = this.container.clientWidth || window.innerWidth;
        const minCell = 480;
        const cols = Math.max(1, Math.floor(width / minCell));
        this.container.style.gridTemplateColumns = `repeat(${Math.min(cols, count)}, 1fr)`;
    }

    destroy() {
        window.removeEventListener('resize', this._boundResize);
        this.rooms.forEach((entry) => entry.cell.remove());
        this.rooms.clear();
    }
}
