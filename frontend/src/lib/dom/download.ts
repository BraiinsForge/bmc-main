export function downloadURL(url: string, suggestedFilename?: Maybe<string>): void {
    const a = document.createElement('a');
    a.href = url;
    a.download = suggestedFilename || '';

    window.document.body.appendChild(a);
    a.click();
    window.document.body.removeChild(a);
}
