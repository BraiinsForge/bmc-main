import { useState, useEffect } from 'react';

export interface TickProps {
    intervalMs?: number;
    render?(value: number): ReactElement;
    children?: ReactNode;
}

export function Tick(props: TickProps) {
    const { intervalMs, render, children } = props;

    const [counter, setCounter] = useState(0);
    useEffect(() => {
        const id = setInterval(() => {
            setCounter(c => c + 1);
        }, intervalMs);
        return () => {
            clearTimeout(id);
        };
    }, [intervalMs]);

    return render?.(counter) ?? children;
}
