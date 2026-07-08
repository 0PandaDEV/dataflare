import { invoke } from '@tauri-apps/api/core'

export const setConnectionsSearch = (value: string) => {
    return invoke('set_connections_search', { value })
}

export const getConnectionsSearch = () => {
    return invoke<string>('get_connections_search')
}
