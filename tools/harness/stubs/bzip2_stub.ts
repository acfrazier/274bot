/**
 * Class stub for #3rdparty/bzip2-wasm — BZip2.ts does `new BZ2Wasm(); await init()`.
 * Pack .dat loads do not need Jagfile decompression when data/pack is already extracted.
 */
export default class BZip2Stub {
    async init() {}
    decompress(_src: Uint8Array, _size?: number, _small?: boolean): Uint8Array {
        throw new Error('bzip2 stub: decompress should not run for pack .dat loads');
    }
    compress(_src: Uint8Array, _a?: boolean, _b?: boolean): Uint8Array {
        throw new Error('bzip2 stub: compress should not run offline');
    }
}
