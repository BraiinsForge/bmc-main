const scrollElements = [document.documentElement, document.body];

const scrollTops = document.querySelectorAll('.scrollTop');
Array.from(scrollTops).forEach(el => {
    el.addEventListener('click', () => {
        try {
            scrollElements.forEach(element => {
                element.scrollTo({ top: 0, left: 0, behavior: 'smooth' });
            });
        } catch (e) {
            console.warn(e);
        }
    });
});

/**
 * @param {HTMLElement} element
 * @returns {void}
 */
function selectNodeContent(element) {
    try {
        const range = document.createRange();
        range.selectNode(element);

        document.getSelection().removeAllRanges();
        document.getSelection().addRange(range);
    } catch (e) {
        console.warn(e);
    }
}

const autoSelects = document.querySelectorAll('.select');
Array.from(autoSelects).forEach(el => {
    el.addEventListener('click', event => {
        try {
            const element = event.target;
            const autoCopy = element.classList.contains('copy');
            const text = element.textContent;

            selectNodeContent(element);
            if (autoCopy) {
                document.execCommand('copy');
                navigator.clipboard.writeText(text);
            }
        } catch (e) {
            console.warn(e);
        }
    });
});
