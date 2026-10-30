// Browser behavior without live network/token dependencies.
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const html = await readFile(new URL('../assets/map-picker.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1].replace('/*TIMESLAY_CONFIG*/null', JSON.stringify({token:'pk.test',query:'coffee',center:[-122.4,37.8],session:7}));
let map, search, marker, response;
const status={textContent:''}, doc={title:'',getElementById:()=>status,createElement:()=>({}),head:{append:s=>s.onload()}};
class Map {
 constructor(options){this.options=options;this.events={};map=this;}
 addControl(){} on(name,fn){this.events[name]=fn;}
 getCenter(){return{lng:-122.5,lat:37.9};}
 queryRenderedFeatures(){return [];}
}
class Search {
 constructor(){search=this;this.events={};}
 addEventListener(name,fn){this.events[name]=fn;}
}
class Marker {
 constructor(){marker=this;}
 setLngLat(c){this.coords=c;return this;} addTo(){return this;} remove(){this.removed=true;}
}
const context={document:doc,MapboxSearchBox:Search,mapboxgl:{supported:()=>true,Map,Marker,NavigationControl:class{}},URL,URLSearchParams,fetch:async()=>response};
vm.runInNewContext(script,context);
await new Promise(resolve=>setImmediate(resolve));
const message=()=>JSON.parse(doc.title.slice('timeslay-map:'.length));
const feature={properties:{mapbox_id:'cafe',name:'Cafe',full_address:'1 Main St'},geometry:{coordinates:[-122.4,37.8]}};
assert.deepEqual(Array.from(map.options.center),[-122.4,37.8]);
assert.equal(search.value,'coffee');
search.events.retrieve({detail:{features:[feature]}});
assert.equal(message().selected.place_id,'cafe');
assert.deepEqual(Array.from(marker.coords),[-122.4,37.8]);
search.events.input();
assert.equal(message().selected,null,'typing invalidates selection');
response={ok:true,json:async()=>({features:[feature]})};
await map.events.click({point:{},lngLat:{lng:-122.4,lat:37.8}});
assert.equal(message().selected.address,'1 Main St','map clicks resolve an address');
let finish;
response={ok:true,json:()=>new Promise(resolve=>{finish=resolve;})};
const pending=map.events.click({point:{},lngLat:{lng:-122.4,lat:37.8}});
await new Promise(resolve=>setImmediate(resolve));
search.events.clear();
finish({features:[feature]});await pending;
assert.equal(message().selected,null,'old reverse lookup cannot override a newer search');
response={ok:false};
await map.events.click({point:{},lngLat:{lng:-122.4,lat:37.8}});
assert.equal(message().selected,null);
assert.match(message().error,/Could not find/);
map.events.moveend();assert.deepEqual(Array.from(search.options.proximity),[-122.5,37.9]);
console.log('Map picker search, selection, clear, click, request race, error and proximity checks passed.');
