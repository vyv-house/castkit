function calcGrid(n) {
    if (n <= 0) return { cols: 1, rows: 1 };
    if (n === 1) return { cols: 1, rows: 1 };
    if (n === 2) return { cols: 2, rows: 1 };
    if (n <= 4) return { cols: 2, rows: 2 };
    if (n <= 6) return { cols: 3, rows: 2 };
    if (n <= 9) return { cols: 3, rows: 3 };
    if (n <= 12) return { cols: 4, rows: 3 };
    const cols = Math.ceil(Math.sqrt(n));
    const rows = Math.ceil(n / cols);
    return { cols, rows };
}

const STYLE_ID = 'monitor-grid-style';

export class MonitorGrid {
    constructor(container) {
        this.container = container;
        this.monitors = new Map();
        this.draggedRoomId = null;
        this.dragOverElement = null;
        this.resizeState = null;
        this.styleElement = this.injectStyles();

        this.container.style.display = 'grid';
        this.container.style.gap = '8px';
        this.container.style.padding = '8px';
        this.container.style.width = '100%';
        this.container.style.height = '100%';

        this.updateLayout();
    }

    addMonitor(roomId) {
        if (this.monitors.has(roomId)) {
            const monitor = this.monitors.get(roomId);
            return { canvas: monitor.canvas, element: monitor.element };
        }

        const element = document.createElement('div');
        element.className = 'monitor-panel';
        element.dataset.roomId = roomId;

        const canvas = document.createElement('canvas');

        const overlay = document.createElement('div');
        overlay.className = 'monitor-overlay';

        const roomName = document.createElement('span');
        roomName.className = 'monitor-room-name';
        roomName.textContent = roomId;

        const fps = document.createElement('span');
        fps.className = 'monitor-fps';
        fps.textContent = '0 fps';

        const resolution = document.createElement('span');
        resolution.className = 'monitor-res';
        resolution.textContent = '--';

        const statusDot = document.createElement('span');
        statusDot.className = 'monitor-status-dot disconnected';

        const resizeHandle = document.createElement('div');
        resizeHandle.className = 'monitor-resize-handle';

        overlay.appendChild(roomName);
        overlay.appendChild(fps);
        overlay.appendChild(resolution);
        overlay.appendChild(statusDot);
        element.appendChild(canvas);
        element.appendChild(overlay);
        element.appendChild(resizeHandle);

        element.addEventListener('pointerdown', (event) => this.startDrag(event, roomId));
        resizeHandle.addEventListener('pointerdown', (event) => this.startResize(event, roomId));
        resizeHandle.addEventListener('dblclick', (event) => this.resetSpan(event, roomId));

        this.container.appendChild(element);
        this.monitors.set(roomId, {
            canvas,
            element,
            fps,
            resolution,
            statusDot,
            colSpan: 1,
            rowSpan: 1,
        });
        this.updateLayout();

        return { canvas, element };
    }

    removeMonitor(roomId) {
        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        monitor.element.remove();
        this.monitors.delete(roomId);
        this.updateLayout();
    }

    setStatus(roomId, status) {
        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        monitor.statusDot.classList.remove('connected', 'reconnecting', 'disconnected');
        monitor.statusDot.classList.add(status);
    }

    setFps(roomId, fps) {
        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        monitor.fps.textContent = `${fps} fps`;
    }

    setResolution(roomId, width, height) {
        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        monitor.resolution.textContent = `${width}×${height}`;
    }

    setError(roomId, message) {
        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        monitor.fps.textContent = 'error';
        monitor.resolution.textContent = message;
    }

    destroy() {
        this.clearDragState();
        this.resizeState = null;
        this.monitors.forEach((monitor) => monitor.element.remove());
        this.monitors.clear();
        if (this.styleElement) {
            this.styleElement.remove();
            this.styleElement = null;
        }
    }

    injectStyles() {
        const style = document.createElement('style');
        style.id = STYLE_ID;
        style.textContent = `
.monitor-panel {
    position: relative;
    background: #0a0a1a;
    border-radius: 8px;
    overflow: hidden;
    contain: strict;
    border: 1px solid rgba(255,255,255,0.06);
    transition: box-shadow 0.2s ease;
    min-height: 0;
    touch-action: none;
}
.monitor-panel canvas {
    width: 100%;
    height: 100%;
    display: block;
    object-fit: contain;
}
.monitor-overlay {
    position: absolute;
    top: 0; left: 0; right: 0;
    padding: 8px 12px;
    display: flex;
    align-items: center;
    gap: 8px;
    background: linear-gradient(to bottom, rgba(0,0,0,0.7), transparent);
    opacity: 0;
    transition: opacity 0.25s ease;
    pointer-events: none;
    font-size: 12px;
    color: #e4e4e4;
}
.monitor-panel:hover .monitor-overlay { opacity: 1; }
.monitor-room-name { font-weight: 700; }
.monitor-fps {
    background: rgba(255,255,255,0.1);
    padding: 2px 6px;
    border-radius: 4px;
}
.monitor-status-dot {
    width: 8px; height: 8px;
    border-radius: 50%;
    background: #2ecc71;
}
.monitor-status-dot.connected { background: #2ecc71; }
.monitor-status-dot.reconnecting { background: #f1c40f; }
.monitor-status-dot.disconnected { background: #e74c3c; }
.monitor-panel.dragging { opacity: 0.5; z-index: 1000; }
.monitor-panel.drag-over { box-shadow: 0 0 0 3px #0f3460; }
.monitor-resize-handle {
    position: absolute;
    bottom: 0; right: 0;
    width: 20px; height: 20px;
    cursor: nwse-resize;
    background: linear-gradient(135deg, transparent 50%, rgba(255,255,255,0.15) 50%);
    touch-action: none;
}`;
        document.head.appendChild(style);
        return style;
    }

    updateLayout() {
        const { cols, rows } = calcGrid(this.monitors.size);
        this.container.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;
        this.container.style.gridTemplateRows = `repeat(${rows}, minmax(0, 1fr))`;
    }

    startDrag(event, roomId) {
        if (event.button !== 0 || event.target.closest('.monitor-resize-handle')) return;

        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        this.draggedRoomId = roomId;
        monitor.element.classList.add('dragging');
        monitor.element.setPointerCapture(event.pointerId);

        const move = (moveEvent) => this.moveDrag(moveEvent);
        const up = (upEvent) => {
            this.endDrag(upEvent);
            monitor.element.releasePointerCapture(upEvent.pointerId);
            monitor.element.removeEventListener('pointermove', move);
            monitor.element.removeEventListener('pointerup', up);
            monitor.element.removeEventListener('pointercancel', up);
        };

        monitor.element.addEventListener('pointermove', move);
        monitor.element.addEventListener('pointerup', up);
        monitor.element.addEventListener('pointercancel', up);
    }

    moveDrag(event) {
        if (!this.draggedRoomId) return;

        const panel = document.elementFromPoint(event.clientX, event.clientY)?.closest('.monitor-panel');
        if (this.dragOverElement && this.dragOverElement !== panel) {
            this.dragOverElement.classList.remove('drag-over');
        }

        const dragged = this.monitors.get(this.draggedRoomId)?.element;
        this.dragOverElement = panel && panel !== dragged && this.container.contains(panel) ? panel : null;

        if (this.dragOverElement) {
            this.dragOverElement.classList.add('drag-over');
        }
    }

    endDrag(event) {
        if (!this.draggedRoomId) return;

        const dragged = this.monitors.get(this.draggedRoomId)?.element;
        const target = document.elementFromPoint(event.clientX, event.clientY)?.closest('.monitor-panel');

        if (dragged && target && target !== dragged && this.container.contains(target)) {
            this.swapPanels(dragged, target);
        }

        this.clearDragState();
    }

    swapPanels(first, second) {
        const firstNext = first.nextSibling;
        const secondNext = second.nextSibling;

        if (firstNext === second) {
            this.container.insertBefore(second, first);
        } else if (secondNext === first) {
            this.container.insertBefore(first, second);
        } else {
            this.container.insertBefore(first, secondNext);
            this.container.insertBefore(second, firstNext);
        }
    }

    clearDragState() {
        this.draggedRoomId = null;
        this.dragOverElement = null;
        this.container.querySelectorAll('.monitor-panel.dragging, .monitor-panel.drag-over').forEach((panel) => {
            panel.classList.remove('dragging', 'drag-over');
        });
    }

    startResize(event, roomId) {
        if (event.button !== 0) return;

        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        event.stopPropagation();
        event.preventDefault();
        const handle = event.currentTarget;
        handle.setPointerCapture(event.pointerId);

        this.resizeState = {
            roomId,
            startX: event.clientX,
            startY: event.clientY,
            startColSpan: monitor.colSpan,
            startRowSpan: monitor.rowSpan,
            cellWidth: this.getCellWidth(),
            cellHeight: this.getCellHeight(),
        };

        const move = (moveEvent) => this.moveResize(moveEvent);
        const up = (upEvent) => {
            this.endResize();
            handle.releasePointerCapture(upEvent.pointerId);
            handle.removeEventListener('pointermove', move);
            handle.removeEventListener('pointerup', up);
            handle.removeEventListener('pointercancel', up);
        };

        handle.addEventListener('pointermove', move);
        handle.addEventListener('pointerup', up);
        handle.addEventListener('pointercancel', up);
    }

    moveResize(event) {
        if (!this.resizeState) return;

        const monitor = this.monitors.get(this.resizeState.roomId);
        if (!monitor) return;

        const colDelta = Math.round((event.clientX - this.resizeState.startX) / this.resizeState.cellWidth);
        const rowDelta = Math.round((event.clientY - this.resizeState.startY) / this.resizeState.cellHeight);
        const { cols, rows } = calcGrid(this.monitors.size);
        const colSpan = Math.min(cols, Math.max(1, this.resizeState.startColSpan + colDelta));
        const rowSpan = Math.min(rows, Math.max(1, this.resizeState.startRowSpan + rowDelta));

        this.applySpan(monitor, colSpan, rowSpan);
    }

    endResize() {
        this.resizeState = null;
    }

    resetSpan(event, roomId) {
        const monitor = this.monitors.get(roomId);
        if (!monitor) return;

        event.stopPropagation();
        this.applySpan(monitor, 1, 1);
    }

    applySpan(monitor, colSpan, rowSpan) {
        monitor.colSpan = colSpan;
        monitor.rowSpan = rowSpan;
        monitor.element.style.gridColumn = `span ${colSpan}`;
        monitor.element.style.gridRow = `span ${rowSpan}`;
    }

    getCellWidth() {
        const styles = getComputedStyle(this.container);
        const columns = styles.gridTemplateColumns.split(' ').filter(Boolean).length || 1;
        return Math.max(1, this.container.clientWidth / columns);
    }

    getCellHeight() {
        const styles = getComputedStyle(this.container);
        const rows = styles.gridTemplateRows.split(' ').filter(Boolean).length || 1;
        return Math.max(1, this.container.clientHeight / rows);
    }
}
