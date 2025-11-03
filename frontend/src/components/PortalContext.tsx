import { createContext } from 'react';

export interface PortalContextValue {
    disablePortal: boolean;
}
export const PortalContext = createContext<PortalContextValue>({
    disablePortal: false,
});
