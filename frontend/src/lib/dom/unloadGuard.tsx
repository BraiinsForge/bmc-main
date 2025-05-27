let isConnected: boolean = false;

function listener(e: BeforeUnloadEvent) {
    // For legacy browsers
    e.returnValue = true;
    e.preventDefault();
}
function enable() {
    window.addEventListener('beforeunload', listener);
    isConnected = true;
}
function disable() {
    window.removeEventListener('beforeunload', listener);
    isConnected = false;
}

export const unloadGuard = {
    enable,
    disable,
    get isEnabled(): boolean {
        return isConnected;
    },
};
