/**
 * ScreenshotSafe Extension — Full-tab pre-upload editor.
 */

(function () {
    'use strict';

    const canvas = document.getElementById('edit-canvas');
    const ctx = canvas.getContext('2d');
    const cropToolBtn = document.getElementById('crop-tool-btn');
    const redactToolBtn = document.getElementById('redact-tool-btn');
    const undoEditBtn = document.getElementById('undo-edit-btn');
    const resetEditBtn = document.getElementById('reset-edit-btn');
    const uploadBtn = document.getElementById('upload-btn');
    const discardBtn = document.getElementById('discard-btn');
    const editorHint = document.getElementById('editor-hint');
    const errorMsg = document.getElementById('error-msg');
    const resultDiv = document.getElementById('result');
    const shareUrlInput = document.getElementById('share-url');
    const rawUrlInput = document.getElementById('raw-url');
    const copyShareBtn = document.getElementById('copy-share-btn');
    const copyRawBtn = document.getElementById('copy-raw-btn');
    const openServerEditorBtn = document.getElementById('open-server-editor-btn');
    const sourceLabel = document.getElementById('source-label');

    const ext = window.sssWebExt;

    let settings = null;
    let currentResult = null;
    let draft = null;
    let activeTool = 'crop';
    let dragStart = null;
    let previewRect = null;
    let editHistory = [];

    init();

    cropToolBtn.addEventListener('click', () => setTool('crop'));
    redactToolBtn.addEventListener('click', () => setTool('redact'));
    undoEditBtn.addEventListener('click', undoEdit);
    resetEditBtn.addEventListener('click', resetEdits);
    uploadBtn.addEventListener('click', uploadEditedScreenshot);
    discardBtn.addEventListener('click', () => window.close());

    canvas.addEventListener('pointerdown', startDrag);
    canvas.addEventListener('pointermove', moveDrag);
    canvas.addEventListener('pointerup', finishDrag);
    canvas.addEventListener('pointercancel', cancelDrag);
    window.addEventListener('resize', () => {
        if (draft) renderEditor();
    });

    copyShareBtn.addEventListener('click', async () => {
        copyShareBtn.textContent = await copyText(shareUrlInput.value) ? '✓' : '!';
        setTimeout(() => { copyShareBtn.textContent = 'Copy'; }, 1500);
    });

    copyRawBtn.addEventListener('click', async () => {
        copyRawBtn.textContent = await copyText(rawUrlInput.value) ? '✓' : '!';
        setTimeout(() => { copyRawBtn.textContent = 'Copy'; }, 1500);
    });

    openServerEditorBtn.addEventListener('click', () => {
        if (currentResult) {
            ext.tabs.create({
                url: `${settings.serverUrl}/screenshots/${currentResult.id}/edit`,
            });
        }
    });

    async function init() {
        try {
            const id = new URLSearchParams(window.location.search).get('id');
            if (!id) {
                throw new Error('Missing screenshot draft. Capture again from the extension popup.');
            }

            settings = await ext.storage.get(['serverUrl', 'apiToken']);
            if (!settings.serverUrl || !settings.apiToken) {
                throw new Error('ScreenshotSafe is not configured. Save your server URL and API token in the extension popup.');
            }

            const response = await ext.runtime.sendMessage({ type: 'sss-get-draft', id });
            if (!response || !response.ok) {
                throw new Error(response && response.error ? response.error : 'Could not load screenshot draft.');
            }

            const image = await loadImage(response.draft.dataUrl);
            draft = {
                image,
                title: response.draft.title || 'Screenshot',
                sourceUrl: response.draft.sourceUrl || '',
                cropRect: null,
                redactions: [],
            };
            sourceLabel.textContent = draft.sourceUrl || draft.title;
            renderEditor();
        } catch (err) {
            showError(err.message);
            uploadBtn.disabled = true;
        }
    }

    async function uploadEditedScreenshot() {
        if (!draft) return;

        hideError();
        uploadBtn.disabled = true;
        uploadBtn.textContent = 'Uploading...';

        try {
            const blob = await renderEditedBlob();
            const result = await uploadBlob(blob);
            currentResult = result;

            shareUrlInput.value = result.share_url;
            rawUrlInput.value = result.raw_url;
            resultDiv.classList.add('show');

            const copied = await copyText(result.share_url);
            uploadBtn.textContent = copied ? 'Uploaded and copied' : 'Uploaded';
        } catch (err) {
            showError(err.message);
            uploadBtn.disabled = false;
            uploadBtn.textContent = 'Upload Edited Screenshot';
        }
    }

    async function uploadBlob(blob) {
        const formData = new FormData();
        formData.append('image', blob, 'screenshot.png');
        formData.append('title', draft.title);
        formData.append('source_url', draft.sourceUrl);

        const resp = await fetch(`${settings.serverUrl}/api/screenshots`, {
            method: 'POST',
            headers: { 'Authorization': `Bearer ${settings.apiToken}` },
            body: formData,
        });

        if (!resp.ok) {
            const errData = await resp.json().catch(() => ({}));
            throw new Error(errData.error || `Upload failed (${resp.status})`);
        }

        return resp.json();
    }

    function renderEditedBlob() {
        const crop = draft.cropRect || {
            x: 0,
            y: 0,
            width: draft.image.naturalWidth,
            height: draft.image.naturalHeight,
        };

        const output = document.createElement('canvas');
        output.width = Math.max(1, Math.round(crop.width));
        output.height = Math.max(1, Math.round(crop.height));
        const outputCtx = output.getContext('2d');

        outputCtx.drawImage(
            draft.image,
            crop.x,
            crop.y,
            crop.width,
            crop.height,
            0,
            0,
            output.width,
            output.height,
        );

        outputCtx.fillStyle = '#000';
        draft.redactions.forEach((rect) => {
            const intersection = intersectRects(rect, crop);
            if (!intersection) return;
            const scaleX = output.width / crop.width;
            const scaleY = output.height / crop.height;
            outputCtx.fillRect(
                (intersection.x - crop.x) * scaleX,
                (intersection.y - crop.y) * scaleY,
                intersection.width * scaleX,
                intersection.height * scaleY,
            );
        });

        return new Promise((resolve, reject) => {
            output.toBlob((blob) => {
                if (blob) {
                    resolve(blob);
                } else {
                    reject(new Error('Could not render edited screenshot'));
                }
            }, 'image/png');
        });
    }

    function resetEdits() {
        if (!draft) return;
        pushHistory();
        draft.cropRect = null;
        draft.redactions = [];
        renderEditor();
    }

    function undoEdit() {
        const previous = editHistory.pop();
        if (!draft || !previous) return;
        draft.cropRect = previous.cropRect;
        draft.redactions = previous.redactions;
        renderEditor();
    }

    function setTool(tool) {
        activeTool = tool;
        cropToolBtn.classList.toggle('active', tool === 'crop');
        redactToolBtn.classList.toggle('active', tool === 'redact');
        editorHint.textContent = tool === 'crop'
            ? 'Drag to set the crop area.'
            : 'Drag over anything sensitive to black it out.';
    }

    function renderEditor() {
        if (!draft) return;

        const bounds = canvas.parentElement.getBoundingClientRect();
        const scale = Math.min(
            Math.max(1, bounds.width - 2) / draft.image.naturalWidth,
            Math.max(1, bounds.height - 2) / draft.image.naturalHeight,
            1,
        );

        canvas.width = Math.max(1, Math.round(draft.image.naturalWidth * scale));
        canvas.height = Math.max(1, Math.round(draft.image.naturalHeight * scale));

        ctx.clearRect(0, 0, canvas.width, canvas.height);
        ctx.drawImage(draft.image, 0, 0, canvas.width, canvas.height);

        draft.redactions.forEach((rect) => drawRect(rect, 'rgba(0,0,0,0.9)', '#000'));

        if (draft.cropRect) {
            shadeOutside(draft.cropRect);
            drawRect(draft.cropRect, 'rgba(0,0,0,0)', '#00cc66', [6, 5]);
        }

        if (previewRect) {
            if (activeTool === 'redact') {
                drawRect(previewRect, 'rgba(0,0,0,0.75)', '#000');
            } else {
                shadeOutside(previewRect);
                drawRect(previewRect, 'rgba(0,0,0,0)', '#00cc66', [6, 5]);
            }
        }
    }

    function startDrag(event) {
        if (!draft) return;
        canvas.setPointerCapture(event.pointerId);
        dragStart = canvasToImagePoint(event);
        previewRect = null;
    }

    function moveDrag(event) {
        if (!draft || !dragStart) return;
        const point = canvasToImagePoint(event);
        previewRect = normalizeRect(dragStart.x, dragStart.y, point.x, point.y);
        renderEditor();
    }

    function finishDrag(event) {
        if (!draft || !dragStart) return;
        const point = canvasToImagePoint(event);
        const rect = normalizeRect(dragStart.x, dragStart.y, point.x, point.y);
        canvas.releasePointerCapture(event.pointerId);
        dragStart = null;
        previewRect = null;

        if (rect.width < 8 || rect.height < 8) {
            renderEditor();
            return;
        }

        pushHistory();
        if (activeTool === 'redact') {
            draft.redactions.push(rect);
        } else {
            draft.cropRect = rect;
        }
        renderEditor();
    }

    function cancelDrag() {
        dragStart = null;
        previewRect = null;
        renderEditor();
    }

    function canvasToImagePoint(event) {
        const bounds = canvas.getBoundingClientRect();
        const scaleX = draft.image.naturalWidth / bounds.width;
        const scaleY = draft.image.naturalHeight / bounds.height;
        return {
            x: clamp((event.clientX - bounds.left) * scaleX, 0, draft.image.naturalWidth),
            y: clamp((event.clientY - bounds.top) * scaleY, 0, draft.image.naturalHeight),
        };
    }

    function imageToCanvasRect(rect) {
        const scaleX = canvas.width / draft.image.naturalWidth;
        const scaleY = canvas.height / draft.image.naturalHeight;
        return {
            x: rect.x * scaleX,
            y: rect.y * scaleY,
            width: rect.width * scaleX,
            height: rect.height * scaleY,
        };
    }

    function drawRect(rect, fill, stroke, dash) {
        const c = imageToCanvasRect(rect);
        ctx.save();
        ctx.fillStyle = fill;
        ctx.strokeStyle = stroke;
        ctx.lineWidth = 2;
        ctx.setLineDash(dash || []);
        ctx.fillRect(c.x, c.y, c.width, c.height);
        ctx.strokeRect(c.x, c.y, c.width, c.height);
        ctx.restore();
    }

    function shadeOutside(rect) {
        const c = imageToCanvasRect(rect);
        ctx.save();
        ctx.fillStyle = 'rgba(0,0,0,0.42)';
        ctx.beginPath();
        ctx.rect(0, 0, canvas.width, canvas.height);
        ctx.rect(c.x, c.y, c.width, c.height);
        ctx.fill('evenodd');
        ctx.restore();
    }

    function normalizeRect(x1, y1, x2, y2) {
        const x = clamp(Math.min(x1, x2), 0, draft.image.naturalWidth);
        const y = clamp(Math.min(y1, y2), 0, draft.image.naturalHeight);
        const maxX = clamp(Math.max(x1, x2), 0, draft.image.naturalWidth);
        const maxY = clamp(Math.max(y1, y2), 0, draft.image.naturalHeight);
        return {
            x,
            y,
            width: maxX - x,
            height: maxY - y,
        };
    }

    function intersectRects(a, b) {
        const x = Math.max(a.x, b.x);
        const y = Math.max(a.y, b.y);
        const right = Math.min(a.x + a.width, b.x + b.width);
        const bottom = Math.min(a.y + a.height, b.y + b.height);
        if (right <= x || bottom <= y) return null;
        return { x, y, width: right - x, height: bottom - y };
    }

    function pushHistory() {
        editHistory.push({
            cropRect: draft.cropRect ? { ...draft.cropRect } : null,
            redactions: draft.redactions.map((rect) => ({ ...rect })),
        });
        if (editHistory.length > 50) {
            editHistory.shift();
        }
    }

    function loadImage(src) {
        return new Promise((resolve, reject) => {
            const image = new Image();
            image.onload = () => resolve(image);
            image.onerror = () => reject(new Error('Could not load captured screenshot'));
            image.src = src;
        });
    }

    function hideError() {
        errorMsg.classList.remove('show');
    }

    function showError(message) {
        errorMsg.textContent = message;
        errorMsg.classList.add('show');
    }

    function clamp(value, min, max) {
        return Math.min(Math.max(value, min), max);
    }

    async function copyText(text) {
        try {
            await navigator.clipboard.writeText(text);
            return true;
        } catch (_) {
            return false;
        }
    }
})();
