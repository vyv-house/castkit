export class JitterBuffer {
    constructor(delayMs = 50) {
        this.delayMs = delayMs;
        this.frames = [];
        this.maxSize = 60;
        this.timeOffset = null;
    }

    push(frame) {
        if (this.timeOffset === null) {
            this.timeOffset = performance.now() - frame.timestamp;
        }
        this.frames.push(frame);
        this.frames.sort((a, b) => a.timestamp - b.timestamp);
        if (this.frames.length > this.maxSize) {
            this.frames = this.frames.slice(-this.maxSize);
        }
    }

    drain(nowMs) {
        if (this.timeOffset === null) return [];
        const threshold = nowMs - this.delayMs - this.timeOffset;
        const ready = [];
        let i = 0;
        while (i < this.frames.length && this.frames[i].timestamp <= threshold) {
            ready.push(this.frames[i]);
            i++;
        }
        if (i > 0) {
            this.frames = this.frames.slice(i);
        }
        return ready;
    }

    clear() {
        this.frames = [];
        this.timeOffset = null;
    }
}
