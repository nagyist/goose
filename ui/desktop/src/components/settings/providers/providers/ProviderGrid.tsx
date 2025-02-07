import React from 'react';
import { ProviderCard } from './subcomponents/ProviderCard';
import ProviderState from './interfaces/ProviderState';
import OnShowModal from './callbacks/ShowModal';
import OnAdd from './callbacks/AddProviderParameters';
import OnDelete from './callbacks/DeleteProviderParameters';
import OnShowSettings from './callbacks/UpdateProviderParameters';
import OnRefresh from './callbacks/RefreshActiveProviders';
import DefaultProviderActions from './subcomponents/actions/DefaultProviderActions';

// Common interfaces and helper functions
interface Provider {
  id: string;
  name: string;
  isConfigured: boolean;
  description: string;
}

function GridLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-3 sm:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-7 gap-3 auto-rows-fr max-w-full [&_*]:z-20">
      {children}
    </div>
  );
}

function ProviderCards({ providers }: { providers: ProviderState[] }) {
  console.log(
    'DefaultProviderActions output:',
    <DefaultProviderActions
      name={providers[0].name}
      isConfigured={providers[0].isConfigured}
      onAdd={OnAdd}
      onDelete={OnDelete}
      onShowSettings={OnShowSettings}
    />
  );
  return (
    <>
      {providers.map((provider) => (
        <ProviderCard
          key={provider.name} // helps React efficiently update and track components when rendering lists
          provider={provider}
          providerCallbacks={{
            onShowModal: OnShowModal,
            onAdd: OnAdd,
            onDelete: OnDelete,
            onShowSettings: OnShowSettings,
            onRefresh: OnRefresh,
          }}
        />
      ))}
    </>
  );
}

export default function ProviderGrid({ providers }: { providers: ProviderState[] }) {
  console.log('got these providers', providers);
  return (
    <GridLayout>
      <ProviderCards providers={providers} />
    </GridLayout>
  );
}
