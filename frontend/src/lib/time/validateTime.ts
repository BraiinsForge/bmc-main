export function validateTime(time: string): boolean {
    const match = time.match(/^([01]\d|2[0-3]):([0-5]\d)$/);
    if (!match) return false;

    const [_, hh, mm] = match;
    const hours = Number(hh);
    const minutes = Number(mm);

    return hours >= 0 && hours <= 23 && minutes >= 0 && minutes <= 59;
}
