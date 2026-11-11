// Windows needs an ICO resource; dx does not convert the shared SVG for it.
// Run `node scripts/windows-icon.mjs` after changing assets/icon/icon.svg.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const sizes = [16, 24, 32, 48, 64, 128, 256];
// Desktop has no adaptive mask, so crop away the mobile safe-zone padding.
const desktopViewBox = "221 160 640 640";
const svg = readFileSync(new URL("../assets/icon/icon.svg", import.meta.url)).toString("base64");
const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  const frames = [];
  for (const size of sizes) {
    const png = await page.evaluate(
      async ({ svg, size, desktopViewBox }) => {
        const svgDocument = new DOMParser().parseFromString(atob(svg), "image/svg+xml");
        svgDocument.documentElement.setAttribute("viewBox", desktopViewBox);
        const image = new Image();
        image.src = `data:image/svg+xml;base64,${btoa(new XMLSerializer().serializeToString(svgDocument))}`;
        await image.decode();
        const canvas = document.createElement("canvas");
        canvas.width = size;
        canvas.height = size;
        canvas.getContext("2d").drawImage(image, 0, 0, size, size);
        return canvas.toDataURL("image/png").split(",")[1];
      },
      { svg, size, desktopViewBox },
    );
    frames.push(Buffer.from(png, "base64"));
  }

  const directory = Buffer.alloc(6 + sizes.length * 16);
  directory.writeUInt16LE(1, 2);
  directory.writeUInt16LE(sizes.length, 4);
  let offset = directory.length;
  for (const [index, size] of sizes.entries()) {
    const entry = 6 + index * 16;
    directory[entry] = size === 256 ? 0 : size;
    directory[entry + 1] = directory[entry];
    directory.writeUInt16LE(1, entry + 4);
    directory.writeUInt16LE(32, entry + 6);
    directory.writeUInt32LE(frames[index].length, entry + 8);
    directory.writeUInt32LE(offset, entry + 12);
    offset += frames[index].length;
  }
  const output = fileURLToPath(new URL("../assets/icon/icon.ico", import.meta.url));
  writeFileSync(output, Buffer.concat([directory, ...frames]));
  console.log(`Created ${output} (${sizes.join(", ")} pixels)`);
} finally {
  await browser.close();
}
