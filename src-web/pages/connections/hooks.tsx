import useSWRImmutable from 'swr/immutable'
import { ClientData, Connection, DatabaseConfig } from '../../tauri'

export const useConnections = () => {
    return useSWRImmutable('connections', () => {
        return ClientData.connectionList()
    })
}

export const useOptions = <T extends DatabaseConfig>(
    data: Connection<T>,
    onChange: React.Dispatch<React.SetStateAction<Connection<T>>>
) => {
    const setName = (newName: string) => {
        onChange((data) => {
            return {
                ...data,
                name: newName
            }
        })
    }

    const setOpt = <K extends keyof T['options']>(key: K, value: T['options'][K]) => {
        onChange((data) => {
            return {
                ...data,
                config: {
                    ...data.config,
                    options: {
                        ...data.config.options,
                        [key]: value
                    }
                }
            }
        })
    }

    return {
        name: data.name,
        options: data.config.options as T['options'],
        setName,
        setOpt
    }
}

