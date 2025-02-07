import React from 'react';
import CardContainer from './CardContainer';
import CardHeader from './CardHeader';
import ProviderState from '../interfaces/ProviderState';
import CardBody from './CardBody';
import ProviderCallbacks from '../interfaces/ConfigurationCallbacks';
import { PROVIDER_REGISTRY, ProviderRegistry } from '../ProviderRegistry';
import ProviderDetails from '../interfaces/ProviderDetails';
import ConfigurationAction from '../interfaces/ConfigurationAction';

interface ProviderCardProps {
  provider: ProviderState;
  providerCallbacks: ProviderCallbacks;
}

export function ProviderCard({ provider, providerCallbacks }: ProviderCardProps) {
  const providerEntry: ProviderRegistry = PROVIDER_REGISTRY.find((p) => p.name === provider.name);
  const providerDetails: ProviderDetails = providerEntry.details;
  console.log('providerDetails', providerDetails);
  const actions: ConfigurationAction[] = providerDetails.getActions(provider, providerCallbacks);
  console.log('got these actions', actions);
  return (
    <CardContainer
      header={
        <CardHeader
          name={providerDetails.name}
          description={providerDetails.description}
          isConfigured={provider.isConfigured}
        />
      }
      body={<CardBody actions={actions} />}
    />
  );
}
