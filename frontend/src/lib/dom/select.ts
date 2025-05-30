export function selectNodeContent(element: Element): void {
    const range = document.createRange();
    range.selectNode(element);

    document.getSelection()?.removeAllRanges();
    document.getSelection()?.addRange(range);
}
