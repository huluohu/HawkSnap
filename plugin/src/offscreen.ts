/**
 * Offscreen document：画布裁剪（MV3 service worker 无 DOM/canvas）。
 *
 * 两件事：'hawksnap:offscreen-crop'（dataUrl + 目标矩形，
 * 设备像素），解码 → drawImage 取区 → toDataURL('image/png') → 回传；
 * 'hawksnap:offscreen-stitch'（整页降级路径）：视口瓦片按设备像素坐标拼成长图。
 * 不做标注/合成（那是编辑器页的事），保持单一职责，离开即释放。
 *
 * 裁剪矩形越界时夹到图像边界（drawImage 对超界源矩形会静默输出空图）。
 */

type CropRequest = {
  type: 'hawksnap:offscreen-crop';
  dataUrl: string;
  sx: number;
  sy: number;
  sw: number;
  sh: number;
};

/** 视口拼接请求（SW 的整页降级路径）：瓦片按设备像素坐标贴到画布上。 */
type StitchRequest = {
  type: 'hawksnap:offscreen-stitch';
  width: number;
  height: number;
  tiles: Array<{ dataUrl: string; x: number; y: number }>;
};

function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v));
}

async function cropToDataUrl(req: CropRequest): Promise<{ ok: boolean; dataUrl?: string; error?: string }> {
  const img = new Image();
  await new Promise<void>((resolve, reject) => {
    img.onload = () => resolve();
    img.onerror = () => reject(new Error('可见区截图解码失败'));
    img.src = req.dataUrl;
  });
  if (!img.naturalWidth || !img.naturalHeight) {
    return { ok: false, error: '可见区截图尺寸为 0' };
  }

  const sx = clamp(Math.round(req.sx), 0, img.naturalWidth - 1);
  const sy = clamp(Math.round(req.sy), 0, img.naturalHeight - 1);
  const sw = clamp(Math.round(req.sw), 1, img.naturalWidth - sx);
  const sh = clamp(Math.round(req.sh), 1, img.naturalHeight - sy);

  const canvas = document.createElement('canvas');
  canvas.width = sw;
  canvas.height = sh;
  const ctx = canvas.getContext('2d');
  if (!ctx) return { ok: false, error: '无法创建 canvas 上下文' };
  ctx.drawImage(img, sx, sy, sw, sh, 0, 0, sw, sh);
  return { ok: true, dataUrl: canvas.toDataURL('image/png') };
}

async function stitchToDataUrl(req: StitchRequest): Promise<{ ok: boolean; dataUrl?: string; error?: string }> {
  const canvas = document.createElement('canvas');
  canvas.width = req.width;
  canvas.height = req.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) return { ok: false, error: '无法创建 canvas 上下文' };
  for (const t of req.tiles) {
    const img = new Image();
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error('视口截图解码失败'));
      img.src = t.dataUrl;
    });
    ctx.drawImage(img, t.x, t.y);
  }
  return { ok: true, dataUrl: canvas.toDataURL('image/png') };
}

chrome.runtime.onMessage.addListener((msg: unknown, _sender, sendResponse) => {
  const m = msg as (CropRequest | StitchRequest) | null;
  if (!m || (m.type !== 'hawksnap:offscreen-crop' && m.type !== 'hawksnap:offscreen-stitch')) {
    return; // 不参与其它消息
  }
  void (async () => {
    try {
      const res =
        m.type === 'hawksnap:offscreen-crop' ? await cropToDataUrl(m) : await stitchToDataUrl(m);
      sendResponse(res);
    } catch (err) {
      sendResponse({ ok: false, error: err instanceof Error ? err.message : String(err) });
    }
  })();
  return true; // 异步 sendResponse
});
