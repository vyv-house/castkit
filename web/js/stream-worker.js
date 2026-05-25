(function () {
    'use strict';

    const HEADER_SIZE = 9;
    const TYPE_KEYFRAME = 0x01;
    const TYPE_DELTA = 0x02;
    const TYPE_META = 0x03;
    const TYPE_FORCE_KEYFRAME = 0x04;
    const COMMAND_KEYFRAME = 'keyframe';
    const MAX_RECONNECT = 30000;
    const MAX_DECODE_QUEUE = 5;
    const MIN_KEYFRAME_REQUEST_INTERVAL_MS = 500;

    let canvas = null;
    let ctx = null;
    let ws = null;
    let decoder = null;
    let currentConfig = null;
    let wsUrl = '';
    let stopped = true;
    let reconnectTimer = null;
    let reconnectDelay = 1000;
    let fpsTimer = null;
    let frameCount = 0;
    let lastFpsTime = performance.now();
    let seenKeyframe = false;
    let needsKeyframe = true;
    let currentWidth = 0;
    let currentHeight = 0;
    let lastKeyframeRequestTime = 0;

    self.onmessage = (event) => {
        const message = event.data || {};

        if (message.type === 'start') {
            start(message);
        } else if (message.type === 'stop') {
            stop();
        }
    };

    function start(message) {
        stop();

        stopped = false;
        canvas = message.canvas;
        wsUrl = message.wsUrl;
        currentWidth = message.width || 0;
        currentHeight = message.height || 0;
        seenKeyframe = false;
        needsKeyframe = true;
        reconnectDelay = 1000;
        frameCount = 0;
        lastFpsTime = performance.now();

        if (!canvas || !wsUrl) {
            postError('Missing canvas or WebSocket URL');
            stop();
            return;
        }

        if (typeof VideoDecoder === 'undefined') {
            postFatal('WebCodecs VideoDecoder is not available in this browser');
            stop();
            return;
        }

        canvas.width = currentWidth;
        canvas.height = currentHeight;
        if (currentWidth && currentHeight) {
            self.postMessage({ type: 'resolution', width: currentWidth, height: currentHeight });
        }
        ctx = canvas.getContext('2d');

        if (!ctx) {
            postFatal('Failed to get 2D canvas context');
            stop();
            return;
        }

        configureDecoder(currentWidth, currentHeight);
        startFpsCounter();
        connect();
    }

    function stop() {
        stopped = true;
        clearReconnectTimer();
        clearFpsCounter();

        if (ws) {
            ws.onopen = null;
            ws.onmessage = null;
            ws.onerror = null;
            ws.onclose = null;
            if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
                ws.close();
            }
            ws = null;
        }

        if (decoder) {
            try {
                decoder.close();
            } catch (err) {
                void err;
            }
            decoder = null;
        }

        currentConfig = null;
        canvas = null;
        ctx = null;
        wsUrl = '';
        seenKeyframe = false;
        needsKeyframe = true;
        currentWidth = 0;
        currentHeight = 0;
        frameCount = 0;
        lastKeyframeRequestTime = 0;
    }

    function connect() {
        if (stopped || !wsUrl) {
            return;
        }

        clearReconnectTimer();

        try {
            ws = new WebSocket(wsUrl);
        } catch (err) {
            postError(err.message || 'Failed to create WebSocket');
            scheduleReconnect();
            return;
        }

        ws.binaryType = 'arraybuffer';

        ws.onopen = () => {
            if (stopped || !ws) {
                return;
            }
            reconnectDelay = 1000;
            resetDecodeQueue(true);
            seenKeyframe = false;
            needsKeyframe = true;
            self.postMessage({ type: 'status', connected: true, reconnecting: false });
            requestKeyframe(true);
        };

        ws.onmessage = (event) => {
            if (stopped || !(event.data instanceof ArrayBuffer)) {
                return;
            }
            handleBinaryMessage(event.data);
        };

        ws.onerror = (event) => {
            const message = event && event.message ? event.message : 'WebSocket error';
            console.error(message, event);
            postError(message);
        };

        ws.onclose = () => {
            ws = null;
            seenKeyframe = false;
            needsKeyframe = true;
            self.postMessage({ type: 'status', connected: false, reconnecting: false });
            if (!stopped) {
                scheduleReconnect();
            }
        };
    }

    function scheduleReconnect() {
        if (stopped || reconnectTimer) {
            return;
        }

        self.postMessage({ type: 'status', connected: false, reconnecting: true });
        reconnectTimer = setTimeout(() => {
            reconnectTimer = null;
            connect();
        }, reconnectDelay);
        reconnectDelay = Math.min(reconnectDelay * 2, MAX_RECONNECT);
    }

    function clearReconnectTimer() {
        if (reconnectTimer) {
            clearTimeout(reconnectTimer);
            reconnectTimer = null;
        }
    }

    function startFpsCounter() {
        clearFpsCounter();
        fpsTimer = setInterval(() => {
            const now = performance.now();
            const elapsed = (now - lastFpsTime) / 1000;
            const fps = elapsed > 0 ? Math.round(frameCount / elapsed) : 0;
            self.postMessage({ type: 'fps', value: fps });
            frameCount = 0;
            lastFpsTime = now;
        }, 1000);
    }

    function clearFpsCounter() {
        if (fpsTimer) {
            clearInterval(fpsTimer);
            fpsTimer = null;
        }
    }

    function handleBinaryMessage(data) {
        if (data.byteLength < HEADER_SIZE) {
            postError('Received packet smaller than header');
            return;
        }

        const view = new DataView(data);
        const frameType = view.getUint8(0);
        const timestamp = view.getUint32(1, false);
        const width = view.getUint16(5, false);
        const height = view.getUint16(7, false);
        const payload = new Uint8Array(data, HEADER_SIZE);

        if (frameType === TYPE_META) {
            handleResolution(width, height);
            return;
        }

        if (frameType === TYPE_FORCE_KEYFRAME) {
            needsKeyframe = true;
            requestKeyframe(true);
            return;
        }

        if (frameType !== TYPE_KEYFRAME && frameType !== TYPE_DELTA) {
            return;
        }

        const isKey = frameType === TYPE_KEYFRAME;

        if (width !== currentWidth || height !== currentHeight) {
            handleResolution(width, height);
            configureDecoder(width, height);
            seenKeyframe = false;
            needsKeyframe = true;
        }

        if (!decoder || !currentConfig) {
            configureDecoder(width, height);
        }

        if (!decoder || !currentConfig) {
            needsKeyframe = true;
            requestKeyframe();
            return;
        }

        if (decoder.state === 'closed') {
            decoder = null;
            currentConfig = null;
            configureDecoder(currentWidth, currentHeight);
            seenKeyframe = false;
            needsKeyframe = true;
            requestKeyframe(true);
            return;
        }

        if (decoder.state !== 'configured') {
            try {
                decoder.configure(currentConfig);
            } catch (err) {
                seenKeyframe = false;
                needsKeyframe = true;
                postError(err.message || 'Failed to reconfigure decoder');
                closeDecoder();
                requestKeyframe();
                return;
            }
        }

        if (!isKey && (!seenKeyframe || needsKeyframe)) {
            return;
        }

        if (decoder.decodeQueueSize > MAX_DECODE_QUEUE) {
            resetDecoderForBackpressure();
            return;
        }

        if (isKey) {
            seenKeyframe = true;
            needsKeyframe = false;
        }

        try {
            const chunk = new EncodedVideoChunk({
                type: isKey ? 'key' : 'delta',
                timestamp: timestamp * 1000,
                data: payload,
            });
            decoder.decode(chunk);
        } catch (err) {
            needsKeyframe = true;
            postError(err.message || 'Failed to decode frame');
            requestKeyframe();
        }
    }

    function configureDecoder(width, height) {
        if (!width || !height) {
            return;
        }

        const nextConfig = {
            codec: 'vp8',
            codedWidth: width,
            codedHeight: height,
        };
        const configChanged = !currentConfig
            || currentConfig.codedWidth !== nextConfig.codedWidth
            || currentConfig.codedHeight !== nextConfig.codedHeight;

        if (!decoder || decoder.state === 'closed') {
            decoder = new VideoDecoder({
                output: (frame) => {
                    if (!ctx || !canvas) {
                        frame.close();
                        return;
                    }
                    ctx.drawImage(frame, 0, 0, canvas.width, canvas.height);
                    frame.close();
                    frameCount++;
                },
                error: (err) => {
                    seenKeyframe = false;
                    needsKeyframe = true;
                    postError(err.message || 'VideoDecoder error');
                    closeDecoder();
                    requestKeyframe();
                },
            });
        }

        try {
            if (configChanged && currentConfig && decoder.state === 'configured') {
                resetDecodeQueue();
            }
            currentConfig = nextConfig;
            decoder.configure(currentConfig);
        } catch (err) {
            seenKeyframe = false;
            needsKeyframe = true;
            postError(err.message || 'Failed to configure decoder');
            closeDecoder();
        }
    }

    function resetDecoderForBackpressure() {
        if (!decoder || !currentConfig) {
            return;
        }

        try {
            decoder.reset();
            decoder.configure(currentConfig);
        } catch (err) {
            postError(err.message || 'Failed to reset decoder');
            closeDecoder();
        }

        seenKeyframe = false;
        needsKeyframe = true;
        requestKeyframe();
    }

    function resetDecodeQueue(reconfigure) {
        if (!decoder || decoder.state === 'closed') {
            return;
        }

        try {
            decoder.reset();
            if (reconfigure && currentConfig) {
                decoder.configure(currentConfig);
            }
        } catch (err) {
            postError(err.message || 'Failed to reset decoder queue');
            closeDecoder();
        }
    }

    function closeDecoder() {
        if (!decoder) {
            return;
        }

        try {
            if (decoder.state !== 'closed') {
                decoder.close();
            }
        } catch (err) {
            void err;
        }
        decoder = null;
        currentConfig = null;
    }

    function handleResolution(width, height) {
        if (!width || !height || (width === currentWidth && height === currentHeight)) {
            return;
        }

        currentWidth = width;
        currentHeight = height;

        if (canvas) {
            canvas.width = width;
            canvas.height = height;
        }

        self.postMessage({ type: 'resolution', width: width, height: height });
    }

    function requestKeyframe(force) {
        const now = performance.now();
        if (!force && now - lastKeyframeRequestTime < MIN_KEYFRAME_REQUEST_INTERVAL_MS) {
            return;
        }

        if (ws && ws.readyState === WebSocket.OPEN) {
            try {
                ws.send(COMMAND_KEYFRAME);
                lastKeyframeRequestTime = now;
            } catch (err) {
                postError(err.message || 'Failed to request keyframe');
            }
        }
    }

    function postError(message) {
        self.postMessage({ type: 'error', message: message });
    }

    function postFatal(message) {
        self.postMessage({ type: 'fatal', message: message });
    }
}());
