await Promise.resolve();

if (!import.meta.main || typeof import.meta.url !== "string" || import.meta.url.length === 0) {
    throw new Error("file-module import.meta was not initialized");
}

function answer() {
    return 42;
}

print(answer());
