// extIds.ts — extension-string interning for the Phase-1 extension prefilter.
//
// Each distinct lowercase extension (the bytes after the final '.' in the
// basename, if any) gets a small stable UInt16 id. The id table is persisted
// alongside the index so a query's extension can be mapped to the same id and
// compared with a single integer instead of a string during Phase-1.
//
// id 0 is reserved for "no extension".

export class ExtTable {
  private map = new Map<string, number>();
  private list: string[] = [""]; // index 0 == no extension

  // Intern an extension string, returning its stable id.
  intern(ext: string): number {
    if (ext === "") return 0;
    const found = this.map.get(ext);
    if (found !== undefined) return found;
    const id = this.list.length;
    if (id > 0xffff) return 0; // overflow: treat as "no extension" (never filters)
    this.list.push(ext);
    this.map.set(ext, id);
    return id;
  }

  // Look up an existing extension id, or 0 if unknown (0 disables the filter).
  idFor(ext: string): number {
    if (ext === "") return 0;
    return this.map.get(ext) ?? 0;
  }

  serialize(): string[] {
    return this.list;
  }

  static deserialize(list: string[]): ExtTable {
    const t = new ExtTable();
    t.list = list.length > 0 ? list.slice() : [""];
    t.map = new Map();
    for (let i = 1; i < t.list.length; i++) t.map.set(t.list[i], i);
    return t;
  }
}

// Extract the lowercase extension from a path's basename bytes.
export function extractExt(pathLower: string, bnStart: number): string {
  const dot = pathLower.lastIndexOf(".");
  if (dot <= bnStart) return ""; // no dot in basename, or leading dot (dotfile)
  return pathLower.slice(dot + 1);
}
