<script lang="ts">
  import { onMount } from 'svelte';

  export interface PrintRoute {
    id: string;
    origin: {
      label: string;
      latitude: number;
      longitude: number;
    };
    cloud: {
      label: string;
      latitude: number;
      longitude: number;
    };
    destination: {
      label: string;
      printer: string;
      latitude: number;
      longitude: number;
    };
    format: 'PDF' | 'RAW';
    elapsed: string;
  }

  const demoRoutes: PrintRoute[] = [
    {
      id: 'job_7K2',
      origin: { label: 'San Francisco', latitude: 37.77, longitude: -122.42 },
      cloud: { label: 'Spool Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Auckland',
        printer: 'Packing station 04',
        latitude: -36.85,
        longitude: 174.76
      },
      format: 'PDF',
      elapsed: '1.4s'
    },
    {
      id: 'job_3M8',
      origin: { label: 'London', latitude: 51.51, longitude: -0.13 },
      cloud: { label: 'Spool Cloud · EU', latitude: 50.11, longitude: 8.68 },
      destination: {
        label: 'Berlin',
        printer: 'Dispatch label 02',
        latitude: 52.52,
        longitude: 13.41
      },
      format: 'RAW',
      elapsed: '0.8s'
    },
    {
      id: 'job_9P1',
      origin: { label: 'Singapore', latitude: 1.35, longitude: 103.82 },
      cloud: { label: 'Spool Cloud · APAC', latitude: -33.87, longitude: 151.21 },
      destination: {
        label: 'Melbourne',
        printer: 'Production floor 01',
        latitude: -37.81,
        longitude: 144.96
      },
      format: 'PDF',
      elapsed: '1.1s'
    },
    {
      id: 'job_4R6',
      origin: { label: 'Toronto', latitude: 43.65, longitude: -79.38 },
      cloud: { label: 'Spool Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Vancouver',
        printer: 'Customer counter 03',
        latitude: 49.28,
        longitude: -123.12
      },
      format: 'PDF',
      elapsed: '1.0s'
    },
    {
      id: 'job_2V5',
      origin: { label: 'Sydney', latitude: -33.87, longitude: 151.21 },
      cloud: { label: 'Spool Cloud · APAC', latitude: 1.35, longitude: 103.82 },
      destination: {
        label: 'Christchurch',
        printer: 'Roastery label station',
        latitude: -43.53,
        longitude: 172.63
      },
      format: 'RAW',
      elapsed: '0.7s'
    },
    {
      id: 'job_6C4',
      origin: { label: 'Paris', latitude: 48.86, longitude: 2.35 },
      cloud: { label: 'Spool Cloud · EU', latitude: 50.11, longitude: 8.68 },
      destination: {
        label: 'Dubai',
        printer: 'Warehouse printer 08',
        latitude: 25.2,
        longitude: 55.27
      },
      format: 'PDF',
      elapsed: '1.2s'
    },
    {
      id: 'job_8F3',
      origin: { label: 'Mumbai', latitude: 19.08, longitude: 72.88 },
      cloud: { label: 'Spool Cloud · APAC', latitude: -33.87, longitude: 151.21 },
      destination: {
        label: 'Singapore',
        printer: 'Fulfilment station 11',
        latitude: 1.35,
        longitude: 103.82
      },
      format: 'RAW',
      elapsed: '0.9s'
    },
    {
      id: 'job_1N7',
      origin: { label: 'São Paulo', latitude: -23.55, longitude: -46.63 },
      cloud: { label: 'Spool Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Lisbon',
        printer: 'Dispatch station 06',
        latitude: 38.72,
        longitude: -9.14
      },
      format: 'PDF',
      elapsed: '1.6s'
    }
  ];

  let {
    routes = demoRoutes,
    simulated = true
  }: {
    routes?: PrintRoute[];
    simulated?: boolean;
  } = $props();

  let stage: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let ready = $state(false);
  let unavailable = $state(false);

  onMount(() => {
    let cancelled = false;
    let dispose = () => {};

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return;
        observer.disconnect();

        void Promise.all([
          import('three'),
          import('topojson-client'),
          import('d3-geo'),
          import('world-atlas/countries-110m.json'),
          import('three/addons/postprocessing/EffectComposer.js'),
          import('three/addons/postprocessing/RenderPass.js'),
          import('three/addons/postprocessing/UnrealBloomPass.js')
        ])
          .then(
            ([
              THREE,
              topojson,
              d3,
              atlasModule,
              { EffectComposer },
              { RenderPass },
              { UnrealBloomPass }
            ]) => {
            if (cancelled) return;

            const context = canvas.getContext('webgl2') ?? canvas.getContext('webgl');
            if (!context) {
              unavailable = true;
              return;
            }

            const renderer = new THREE.WebGLRenderer({
              canvas,
              context,
              antialias: true,
              alpha: true,
              powerPreference: 'high-performance'
            });
            renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.75));
            renderer.setClearColor(0x090b10, 0);
            renderer.outputColorSpace = THREE.SRGBColorSpace;
            renderer.toneMapping = THREE.ACESFilmicToneMapping;
            renderer.toneMappingExposure = 1.18;

            const scene = new THREE.Scene();
            scene.fog = new THREE.FogExp2(0x090b10, 0.035);

            const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 100);
            camera.position.set(0, 0.05, 5.05);

            const composer = new EffectComposer(renderer);
            composer.addPass(new RenderPass(scene, camera));
            const bloomPass = new UnrealBloomPass(
              new THREE.Vector2(stage.clientWidth, stage.clientHeight),
              1.08,
              0.34,
              0.92
            );
            composer.addPass(bloomPass);

            const world = new THREE.Group();
            world.rotation.set(-0.12, -0.1, 0.02);
            scene.add(world);

            const globe = new THREE.Mesh(
              new THREE.SphereGeometry(1.72, 96, 96),
              new THREE.ShaderMaterial({
                uniforms: {
                  oceanDark: { value: new THREE.Color(0x010513) },
                  oceanLight: { value: new THREE.Color(0x10123b) },
                  rimColor: { value: new THREE.Color(0x087c8c) }
                },
                vertexShader: `
                  varying vec3 vNormal;
                  varying vec3 vViewPosition;
                  varying vec3 vObjectPosition;
                  void main() {
                    vec4 viewPosition = modelViewMatrix * vec4(position, 1.0);
                    vNormal = normalize(normalMatrix * normal);
                    vViewPosition = -viewPosition.xyz;
                    vObjectPosition = position;
                    gl_Position = projectionMatrix * viewPosition;
                  }
                `,
                fragmentShader: `
                  uniform vec3 oceanDark;
                  uniform vec3 oceanLight;
                  uniform vec3 rimColor;
                  varying vec3 vNormal;
                  varying vec3 vViewPosition;
                  varying vec3 vObjectPosition;

                  float random(vec2 coordinate) {
                    return fract(sin(dot(coordinate, vec2(12.9898, 78.233))) * 43758.5453);
                  }

                  void main() {
                    vec3 normal = normalize(vNormal);
                    vec3 viewDirection = normalize(vViewPosition);
                    vec3 lightDirection = normalize(vec3(-0.65, 0.48, 0.78));
                    float diffuse = max(dot(normal, lightDirection), 0.0);
                    float fresnel = pow(1.0 - max(dot(normal, viewDirection), 0.0), 2.4);
                    float latitudeShade = smoothstep(-1.65, 1.6, vObjectPosition.y);
                    float grain = random(
                      vObjectPosition.xy * 913.7 +
                      vec2(vObjectPosition.z * 417.3, vObjectPosition.z * 271.9)
                    ) - 0.5;

                    vec3 ocean = mix(oceanDark, oceanLight, diffuse * 0.7 + latitudeShade * 0.08);
                    ocean += rimColor * fresnel * 0.19;
                    ocean += grain * 0.028;
                    gl_FragColor = vec4(ocean, 1.0);
                  }
                `
              })
            );
            world.add(globe);

            const atmosphere = new THREE.Mesh(
              new THREE.SphereGeometry(1.82, 64, 64),
              new THREE.ShaderMaterial({
                transparent: true,
                side: THREE.BackSide,
                blending: THREE.AdditiveBlending,
                depthWrite: false,
                uniforms: {
                  glowColor: { value: new THREE.Color(0x006aff) }
                },
                vertexShader: `
                  varying vec3 vNormal;
                  varying vec3 vPositionNormal;
                  void main() {
                    vNormal = normalize(normalMatrix * normal);
                    vPositionNormal = normalize((modelViewMatrix * vec4(position, 1.0)).xyz);
                    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
                  }
                `,
                fragmentShader: `
                  uniform vec3 glowColor;
                  varying vec3 vNormal;
                  varying vec3 vPositionNormal;
                  void main() {
                    float fresnel = 0.72 - dot(vNormal, vPositionNormal);
                    float intensity = pow(max(fresnel, 0.0), 2.15);
                    gl_FragColor = vec4(glowColor, intensity * 0.68);
                  }
                `
              })
            );
            world.add(atmosphere);

            const innerGlow = new THREE.Mesh(
              new THREE.SphereGeometry(1.708, 72, 72),
              new THREE.ShaderMaterial({
                transparent: true,
                blending: THREE.AdditiveBlending,
                depthWrite: false,
                uniforms: {
                  highColor: { value: new THREE.Color(0x0878ff) },
                  lowColor: { value: new THREE.Color(0x36c98a) }
                },
                vertexShader: `
                  varying vec3 vNormal;
                  varying vec3 vWorldPosition;
                  void main() {
                    vNormal = normalize(normalMatrix * normal);
                    vWorldPosition = position;
                    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
                  }
                `,
                fragmentShader: `
                  uniform vec3 highColor;
                  uniform vec3 lowColor;
                  varying vec3 vNormal;
                  varying vec3 vWorldPosition;
                  void main() {
                    float rim = pow(1.0 - abs(vNormal.z), 3.2);
                    float vertical = smoothstep(-1.7, 1.7, vWorldPosition.y);
                    vec3 tint = mix(lowColor, highColor, vertical);
                    gl_FragColor = vec4(tint, rim * 0.16);
                  }
                `
              })
            );
            world.add(innerGlow);

            const latitudeLongitudeToVector = (
              latitude: number,
              longitude: number,
              radius = 1.74
            ) => {
              const phi = ((90 - latitude) * Math.PI) / 180;
              const theta = ((longitude + 180) * Math.PI) / 180;
              return new THREE.Vector3(
                -radius * Math.sin(phi) * Math.cos(theta),
                radius * Math.cos(phi),
                radius * Math.sin(phi) * Math.sin(theta)
              );
            };

            const atlas = atlasModule.default;
            const landFeature = topojson.feature(
              atlas as never,
              atlas.objects.land as never
            );

            const maskCanvas = document.createElement('canvas');
            maskCanvas.width = 720;
            maskCanvas.height = 360;
            const maskContext = maskCanvas.getContext('2d');
            if (!maskContext) throw new Error('Unable to create the globe land mask');
            const projection = d3
              .geoEquirectangular()
              .translate([maskCanvas.width / 2, maskCanvas.height / 2])
              .scale(maskCanvas.width / (2 * Math.PI));
            const path = d3.geoPath(projection, maskContext);
            maskContext.fillStyle = '#fff';
            maskContext.beginPath();
            path(landFeature);
            maskContext.fill();
            const mask = maskContext.getImageData(
              0,
              0,
              maskCanvas.width,
              maskCanvas.height
            ).data;
            const isLand = (latitude: number, longitude: number) => {
              const x = Math.max(
                0,
                Math.min(maskCanvas.width - 1, Math.floor(((longitude + 180) / 360) * maskCanvas.width))
              );
              const y = Math.max(
                0,
                Math.min(maskCanvas.height - 1, Math.floor(((90 - latitude) / 180) * maskCanvas.height))
              );
              return (mask[(y * maskCanvas.width + x) * 4 + 3] ?? 0) > 0;
            };

            const landPositions: number[] = [];
            const lightPositions: number[] = [];
            for (let latitude = -60; latitude <= 84; latitude += 0.68) {
              for (let longitude = -179.5; longitude <= 179.5; longitude += 0.75) {
                if (!isLand(latitude, longitude)) continue;
                const latitudeJitter = Math.sin(longitude * 4.21 + latitude * 1.77) * 0.18;
                const longitudeJitter = Math.cos(latitude * 3.93 - longitude * 1.31) * 0.18;
                const point = latitudeLongitudeToVector(
                  latitude + latitudeJitter,
                  longitude + longitudeJitter,
                  1.739
                );
                landPositions.push(point.x, point.y, point.z);
                const lightNoise =
                  Math.sin(longitude * 8.13 + latitude * 3.71) +
                  Math.cos(longitude * 2.37 - latitude * 6.19);
                if (lightNoise > 1.72 && Math.abs(latitude) < 68) {
                  const light = latitudeLongitudeToVector(latitude, longitude, 1.754);
                  lightPositions.push(light.x, light.y, light.z);
                }
              }
            }

            const lightHubs: Array<[number, number, number]> = [
              [40.71, -74.01, 54],
              [34.05, -118.24, 46],
              [41.88, -87.63, 30],
              [19.43, -99.13, 35],
              [-23.55, -46.63, 44],
              [51.51, -0.13, 45],
              [48.86, 2.35, 34],
              [50.11, 8.68, 28],
              [52.52, 13.41, 27],
              [41.9, 12.5, 24],
              [6.52, 3.38, 28],
              [25.2, 55.27, 31],
              [19.08, 72.88, 42],
              [28.61, 77.21, 44],
              [1.35, 103.82, 35],
              [22.32, 114.17, 33],
              [31.23, 121.47, 48],
              [35.68, 139.69, 50],
              [37.57, 126.98, 39],
              [-33.87, 151.21, 35],
              [-37.81, 144.96, 28],
              [-36.85, 174.76, 20],
              [-43.53, 172.63, 16]
            ];
            lightHubs.forEach(([latitude, longitude, count], hubIndex) => {
              for (let index = 0; index < count; index += 1) {
                const angle = index * 2.399963 + hubIndex * 0.71;
                const distance = Math.sqrt((index + 0.5) / count) * 2.2;
                const lightLatitude = latitude + Math.sin(angle) * distance;
                const lightLongitude =
                  longitude +
                  (Math.cos(angle) * distance) /
                    Math.max(0.35, Math.cos((latitude * Math.PI) / 180));
                const light = latitudeLongitudeToVector(
                  lightLatitude,
                  lightLongitude,
                  1.756 + (index % 4) * 0.001
                );
                lightPositions.push(light.x, light.y, light.z);
              }
            });

            const makeDotMaterial = (
              color: number,
              pointSize: number,
              opacity: number,
              additive = false,
              intensity = 1,
              pulseAmount = 0
            ) =>
              new THREE.ShaderMaterial({
                transparent: true,
                depthWrite: false,
                blending: additive ? THREE.AdditiveBlending : THREE.NormalBlending,
                uniforms: {
                  dotColor: { value: new THREE.Color(color) },
                  dotSize: { value: pointSize * renderer.getPixelRatio() },
                  dotOpacity: { value: opacity },
                  dotIntensity: { value: intensity },
                  dotTime: { value: 0 },
                  pulseAmount: { value: pulseAmount }
                },
                vertexShader: `
                  uniform float dotSize;
                  varying float vNoise;
                  void main() {
                    vec4 viewPosition = modelViewMatrix * vec4(position, 1.0);
                    vNoise = fract(
                      sin(dot(position.xy + position.zz, vec2(41.73, 289.11))) * 27419.53
                    );
                    gl_PointSize = dotSize / max(0.7, -viewPosition.z);
                    gl_Position = projectionMatrix * viewPosition;
                  }
                `,
                fragmentShader: `
                  uniform vec3 dotColor;
                  uniform float dotOpacity;
                  uniform float dotIntensity;
                  uniform float dotTime;
                  uniform float pulseAmount;
                  varying float vNoise;
                  void main() {
                    float distanceFromCenter = length(gl_PointCoord - vec2(0.5)) * 2.0;
                    if (distanceFromCenter > 1.0) discard;
                    float halo = exp(-3.8 * distanceFromCenter * distanceFromCenter);
                    float core = smoothstep(0.32, 0.0, distanceFromCenter);
                    float pulse =
                      1.0 + sin(dotTime * 1.4 + vNoise * 12.566) * pulseAmount;
                    gl_FragColor = vec4(
                      dotColor * dotIntensity * (0.72 + vNoise * 0.46) * pulse,
                      (halo * 0.62 + core) * dotOpacity
                    );
                  }
                `
              });

            const landGeometry = new THREE.BufferGeometry();
            landGeometry.setAttribute(
              'position',
              new THREE.Float32BufferAttribute(landPositions, 3)
            );
            const landPoints = new THREE.Points(
              landGeometry,
              makeDotMaterial(0x2bc69d, 5.2, 0.7, false, 0.72)
            );
            world.add(landPoints);

            const lightGeometry = new THREE.BufferGeometry();
            lightGeometry.setAttribute(
              'position',
              new THREE.Float32BufferAttribute(lightPositions, 3)
            );
            const cityLightMaterial = makeDotMaterial(0xffe3aa, 15, 0.94, true, 1.85, 0.13);
            const cityLights = new THREE.Points(lightGeometry, cityLightMaterial);
            world.add(cityLights);

            const countryMesh = topojson.mesh(
              atlas as never,
              atlas.objects.countries as never,
              (left, right) => left !== right
            ) as { coordinates: number[][][] };
            const borderPositions: number[] = [];
            countryMesh.coordinates.forEach((line) => {
              for (let index = 1; index < line.length; index += 1) {
                const previous = line[index - 1];
                const current = line[index];
                if (
                  !previous ||
                  !current ||
                  previous[0] === undefined ||
                  previous[1] === undefined ||
                  current[0] === undefined ||
                  current[1] === undefined ||
                  Math.abs(previous[0] - current[0]) > 180
                ) {
                  continue;
                }
                const start = latitudeLongitudeToVector(previous[1], previous[0], 1.746);
                const end = latitudeLongitudeToVector(current[1], current[0], 1.746);
                borderPositions.push(start.x, start.y, start.z, end.x, end.y, end.z);
              }
            });
            const countryGeometry = new THREE.BufferGeometry();
            countryGeometry.setAttribute(
              'position',
              new THREE.Float32BufferAttribute(borderPositions, 3)
            );
            const countryBorders = new THREE.LineSegments(
              countryGeometry,
              new THREE.LineBasicMaterial({
                color: 0x79e5c2,
                transparent: true,
                opacity: 0.3,
                blending: THREE.AdditiveBlending,
                depthWrite: false
              })
            );
            world.add(countryBorders);

            const stars: number[] = [];
            for (let index = 0; index < 720; index += 1) {
              const radius = 8 + ((index * 37) % 100) / 28;
              const phi = Math.acos(1 - (2 * (index + 0.5)) / 720);
              const theta = Math.PI * (1 + Math.sqrt(5)) * index;
              stars.push(
                radius * Math.sin(phi) * Math.cos(theta),
                radius * Math.cos(phi),
                radius * Math.sin(phi) * Math.sin(theta)
              );
            }
            const starGeometry = new THREE.BufferGeometry();
            starGeometry.setAttribute('position', new THREE.Float32BufferAttribute(stars, 3));
            const starField = new THREE.Points(
              starGeometry,
              new THREE.PointsMaterial({
                color: 0x8bb7eb,
                size: 0.014,
                transparent: true,
                opacity: 0.32,
                depthWrite: false
              })
            );
            scene.add(starField);

            const visibleRoutes = routes.length > 0 ? routes : demoRoutes;
            const preferredSequence = [0, 4, 2, 7, 1, 5, 3, 6];
            const routeSequence = [
              ...preferredSequence.filter((index) => index < visibleRoutes.length),
              ...visibleRoutes
                .map((_, index) => index)
                .filter((index) => !preferredSequence.includes(index))
            ];
            const sequencePosition = new Map(
              routeSequence.map((routeIndex, index) => [routeIndex, index])
            );
            const routeVisuals = visibleRoutes.map((route, index) => {
              const group = new THREE.Group();
              const origin = latitudeLongitudeToVector(
                route.origin.latitude,
                route.origin.longitude
              );
              const cloud = latitudeLongitudeToVector(
                route.cloud.latitude,
                route.cloud.longitude
              );
              const destination = latitudeLongitudeToVector(
                route.destination.latitude,
                route.destination.longitude
              );

              const makeCurve = (
                start: InstanceType<typeof THREE.Vector3>,
                end: InstanceType<typeof THREE.Vector3>,
                arcBias: number
              ) => {
                const startDirection = start.clone().normalize();
                const endDirection = end.clone().normalize();
                const angle = Math.max(0.001, startDirection.angleTo(endDirection));
                const axis = startDirection.clone().cross(endDirection);
                if (axis.lengthSq() < 0.000001) axis.set(0, 1, 0);
                axis.normalize();

                const points = Array.from({ length: 49 }, (_, index) => {
                  const progress = index / 48;
                  const direction = startDirection
                    .clone()
                    .applyAxisAngle(axis, angle * progress);
                  const altitude =
                    Math.sin(Math.PI * progress) * (0.13 + angle * 0.27 + arcBias);
                  return direction.multiplyScalar(1.75 + altitude);
                });
                return new THREE.CatmullRomCurve3(points, false, 'centripetal');
              };

              const inbound = makeCurve(origin, cloud, 0.01);
              const outbound = makeCurve(cloud, destination, 0.035);
              const inboundPoints = inbound.getPoints(96);
              const outboundPoints = outbound.getPoints(96);
              const inboundGeometry = new THREE.BufferGeometry().setFromPoints(inboundPoints);
              const outboundGeometry = new THREE.BufferGeometry().setFromPoints(outboundPoints);
              inboundGeometry.setDrawRange(0, 0);
              outboundGeometry.setDrawRange(0, 0);
              const lineMaterials = [
                new THREE.LineBasicMaterial({
                  color: new THREE.Color(0xe8f3ff).multiplyScalar(1.65),
                  transparent: true,
                  opacity: 0,
                  blending: THREE.AdditiveBlending,
                  depthWrite: false
                }),
                new THREE.LineBasicMaterial({
                  color: new THREE.Color(0xffffff).multiplyScalar(1.72),
                  transparent: true,
                  opacity: 0,
                  blending: THREE.AdditiveBlending,
                  depthWrite: false
                })
              ];

              group.add(
                new THREE.Line(inboundGeometry, lineMaterials[0]),
                new THREE.Line(outboundGeometry, lineMaterials[1])
              );

              const nodes = [
                [origin, 0xd9ecff, 0.026],
                [cloud, 0x5ca3ff, 0.034],
                [destination, 0xffdf9c, 0.03]
              ] as const;
              const nodeMaterials: InstanceType<typeof THREE.MeshBasicMaterial>[] = [];
              nodes.forEach(([position, color, size]) => {
                const material = new THREE.MeshBasicMaterial({
                  color: new THREE.Color(color).multiplyScalar(1.7),
                  transparent: true,
                  opacity: 0,
                  blending: THREE.AdditiveBlending,
                  depthWrite: false
                });
                const node = new THREE.Mesh(
                  new THREE.SphereGeometry(size, 16, 16),
                  material
                );
                node.position.copy(position);
                nodeMaterials.push(material);
                group.add(node);
              });

              const particleMaterial = new THREE.MeshBasicMaterial({
                color: new THREE.Color(0xffffff).multiplyScalar(2),
                transparent: true,
                opacity: 0,
                blending: THREE.AdditiveBlending,
                depthWrite: false
              });
              const particle = new THREE.Mesh(
                new THREE.SphereGeometry(0.021, 12, 12),
                particleMaterial
              );
              particle.position.copy(inbound.getPoint(0));
              group.add(particle);
              world.add(group);

              return {
                group,
                inboundGeometry,
                outboundGeometry,
                inboundPointCount: inboundPoints.length,
                outboundPointCount: outboundPoints.length,
                lineMaterials,
                nodeMaterials,
                particle,
                particleMaterial,
                inbound,
                outbound,
                sequencePosition: sequencePosition.get(index) ?? index
              };
            });

            let targetRotationX = world.rotation.x;
            let targetRotationY = world.rotation.y;
            let dragging = false;
            let previousX = 0;
            let previousY = 0;
            let animationFrame = 0;
            const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
            const clamp = (value: number, minimum = 0, maximum = 1) =>
              Math.min(maximum, Math.max(minimum, value));
            const smoothstep = (minimum: number, maximum: number, value: number) => {
              const progress = clamp((value - minimum) / Math.max(maximum - minimum, 0.0001));
              return progress * progress * (3 - 2 * progress);
            };
            const routeStagger = 0.78;
            const routeLifetime = 3.7;
            const routeCycle =
              Math.max(routeSequence.length, 1) * routeStagger + routeLifetime + 0.8;

            const updateRouteVisuals = (elapsed: number) => {
              routeVisuals.forEach((visual) => {
                if (reducedMotion) {
                  const shown = visual.sequencePosition < Math.min(4, routeVisuals.length);
                  visual.inboundGeometry.setDrawRange(
                    0,
                    shown ? visual.inboundPointCount : 0
                  );
                  visual.outboundGeometry.setDrawRange(
                    0,
                    shown ? visual.outboundPointCount : 0
                  );
                  visual.lineMaterials.forEach((material) => {
                    material.opacity = shown ? 0.28 : 0;
                  });
                  visual.nodeMaterials.forEach((material) => {
                    material.opacity = shown ? 0.66 : 0;
                  });
                  visual.particle.visible = false;
                  return;
                }

                const start = visual.sequencePosition * routeStagger;
                const localTime = ((elapsed - start) % routeCycle + routeCycle) % routeCycle;
                const inboundProgress = smoothstep(0, 0.76, localTime);
                const outboundProgress = smoothstep(0.54, 1.34, localTime);
                const fade = 1 - smoothstep(2.15, routeLifetime, localTime);
                const visible = localTime <= routeLifetime;
                const opacity = visible ? fade : 0;

                visual.inboundGeometry.setDrawRange(
                  0,
                  Math.ceil(visual.inboundPointCount * inboundProgress)
                );
                visual.outboundGeometry.setDrawRange(
                  0,
                  Math.ceil(visual.outboundPointCount * outboundProgress)
                );
                visual.lineMaterials[0]!.opacity = opacity * 0.7;
                visual.lineMaterials[1]!.opacity = opacity * 0.76;
                visual.nodeMaterials[0]!.opacity = opacity * smoothstep(0, 0.16, localTime);
                visual.nodeMaterials[1]!.opacity =
                  opacity * smoothstep(0.52, 0.78, localTime) * 0.95;
                visual.nodeMaterials[2]!.opacity =
                  opacity * smoothstep(1.08, 1.36, localTime) * 0.95;

                const drawingInbound = localTime <= 0.76;
                const drawingOutbound = localTime > 0.54 && localTime <= 1.34;
                visual.particle.visible = visible && (drawingInbound || drawingOutbound);
                visual.particleMaterial.opacity = opacity * 0.98;
                if (drawingInbound) {
                  visual.particle.position.copy(
                    visual.inbound.getPoint(clamp(localTime / 0.76))
                  );
                } else if (drawingOutbound) {
                  visual.particle.position.copy(
                    visual.outbound.getPoint(clamp((localTime - 0.54) / 0.8))
                  );
                }
                visual.particle.scale.setScalar(
                  0.88 + Math.sin(elapsed * 8.4 + visual.sequencePosition) * 0.12
                );
              });
            };

            const render = () => composer.render();
            const resize = () => {
              const width = stage.clientWidth;
              const height = stage.clientHeight;
              camera.position.z = width < 600 ? 6.15 : 5.05;
              world.position.x = width < 900 ? 0 : -1.08;
              world.position.y = width < 600 ? -0.82 : 0;
              renderer.setSize(width, height, false);
              composer.setSize(width, height);
              camera.aspect = width / Math.max(height, 1);
              camera.updateProjectionMatrix();
              render();
            };

            let previousFrame = performance.now();
            let elapsed = 0;
            const animate = (now = performance.now()) => {
              const delta = Math.min((now - previousFrame) / 1000, 0.04);
              previousFrame = now;
              elapsed += delta;
              if (!dragging) targetRotationY += delta * 0.075;
              world.rotation.x += (targetRotationX - world.rotation.x) * 0.07;
              world.rotation.y += (targetRotationY - world.rotation.y) * 0.07;

              updateRouteVisuals(elapsed);

              cityLightMaterial.uniforms.dotTime!.value = elapsed;
              camera.position.x = Math.sin(elapsed * 0.08) * 0.08;
              camera.position.y = 0.05 + Math.cos(elapsed * 0.11) * 0.045;
              camera.lookAt(0, 0, 0);
              starField.rotation.y -= delta * 0.006;
              render();
              animationFrame = requestAnimationFrame(animate);
            };

            const handlePointerDown = (event: PointerEvent) => {
              dragging = true;
              previousX = event.clientX;
              previousY = event.clientY;
              canvas.setPointerCapture(event.pointerId);
              canvas.classList.add('is-dragging');
            };
            const handlePointerMove = (event: PointerEvent) => {
              if (!dragging) return;
              targetRotationY += (event.clientX - previousX) * 0.006;
              targetRotationX = Math.max(
                -0.65,
                Math.min(0.55, targetRotationX + (event.clientY - previousY) * 0.004)
              );
              previousX = event.clientX;
              previousY = event.clientY;
              if (reducedMotion) {
                world.rotation.x = targetRotationX;
                world.rotation.y = targetRotationY;
                render();
              }
            };
            const handlePointerUp = (event: PointerEvent) => {
              dragging = false;
              if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
              canvas.classList.remove('is-dragging');
            };

            canvas.addEventListener('pointerdown', handlePointerDown);
            canvas.addEventListener('pointermove', handlePointerMove);
            canvas.addEventListener('pointerup', handlePointerUp);
            canvas.addEventListener('pointercancel', handlePointerUp);

            const resizeObserver = new ResizeObserver(resize);
            resizeObserver.observe(stage);
            updateRouteVisuals(0);
            resize();
            ready = true;

            if (!reducedMotion) animationFrame = requestAnimationFrame(animate);

            dispose = () => {
              cancelAnimationFrame(animationFrame);
              resizeObserver.disconnect();
              canvas.removeEventListener('pointerdown', handlePointerDown);
              canvas.removeEventListener('pointermove', handlePointerMove);
              canvas.removeEventListener('pointerup', handlePointerUp);
              canvas.removeEventListener('pointercancel', handlePointerUp);
              routeVisuals.forEach((visual) => {
                visual.group.traverse((object) => {
                  if (object instanceof THREE.Mesh || object instanceof THREE.Line) {
                    object.geometry.dispose();
                    const materials = Array.isArray(object.material)
                      ? object.material
                      : [object.material];
                    materials.forEach((material) => material.dispose());
                  }
                });
              });
              globe.geometry.dispose();
              (globe.material as InstanceType<typeof THREE.Material>).dispose();
              atmosphere.geometry.dispose();
              atmosphere.material.dispose();
              innerGlow.geometry.dispose();
              innerGlow.material.dispose();
              landGeometry.dispose();
              landPoints.material.dispose();
              lightGeometry.dispose();
              cityLights.material.dispose();
              countryGeometry.dispose();
              countryBorders.material.dispose();
              starGeometry.dispose();
              starField.material.dispose();
              bloomPass.dispose();
              composer.dispose();
              renderer.dispose();
            };
          })
          .catch(() => {
            unavailable = true;
          });
      },
      { rootMargin: '180px' }
    );

    observer.observe(stage);

    return () => {
      cancelled = true;
      observer.disconnect();
      dispose();
    };
  });
</script>

<figure class="network">
  <div
    class="render"
    bind:this={stage}
    role="img"
    aria-label="Interactive three-dimensional globe illustrating print jobs travelling through Spool to local printers"
  >
    <canvas
      bind:this={canvas}
      class:visible={ready}
      aria-hidden="true"
    ></canvas>
    {#if unavailable}
      <span class="render-unavailable">Interactive globe unavailable</span>
    {/if}

    <div class="demo-label">
      <i></i>
      <span>{simulated ? 'Network preview' : 'Live print network'}</span>
      {#if !unavailable}<small>Drag to explore</small>{/if}
    </div>

    <div class="queue-card">
      <span><i></i> Durable by design</span>
      <strong>Work waits safely.</strong>
      <small>Cloud and local queues recover through interruptions.</small>
    </div>

    <div class="capability-card">
      <span><i></i> Full print capability</span>
      <strong>Your drivers stay in control.</strong>
      <small>Paper, trays, finishing, and vendor settings stay local.</small>
    </div>
  </div>

  <figcaption>Illustrative network activity. The component is ready to receive live telemetry.</figcaption>
</figure>

<style>
  .network {
    position: relative;
    height: 100%;
    overflow: hidden;
    margin: 0;
    background: transparent;
  }
  .render {
    position: relative;
    height: 100%;
    min-height: 860px;
    overflow: hidden;
    isolation: isolate;
    background:
      radial-gradient(circle at 48% 46%, rgb(0 106 255 / 0.19), transparent 46%),
      transparent;
  }
  .render::after {
    position: absolute;
    z-index: 0;
    inset: 0;
    background:
      radial-gradient(circle at 48% 48%, transparent 34%, rgb(2 11 22 / 0.08) 66%, rgb(2 11 22 / 0.58) 100%);
    content: '';
    pointer-events: none;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  canvas {
    z-index: 0;
    display: block;
    cursor: grab;
    opacity: 0;
    touch-action: pan-y;
    transition: opacity 500ms ease;
  }
  canvas.visible { opacity: 1; }
  :global(canvas.is-dragging) { cursor: grabbing; }
  .render-unavailable {
    position: absolute;
    z-index: 2;
    inset: 50% auto auto 50%;
    color: #77918c;
    font: 10px var(--font-mono);
    transform: translate(-50%, -50%);
  }
  .demo-label,
  .queue-card,
  .capability-card {
    position: absolute;
    z-index: 2;
    border: 1px solid rgb(255 255 255 / 0.14);
    background: rgb(4 14 29 / 0.8);
    color: white;
    box-shadow: 0 20px 45px rgb(0 0 0 / 0.28);
    backdrop-filter: blur(18px);
    pointer-events: none;
  }
  .demo-label {
    bottom: 22px;
    left: 22px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 12px;
    border-radius: 999px;
    font: 10px var(--font-mono);
  }
  .demo-label i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #5ca3ff;
    box-shadow: 0 0 0 5px rgb(92 163 255 / 0.13);
  }
  .demo-label small {
    padding-left: 8px;
    border-left: 1px solid rgb(255 255 255 / 0.14);
    color: #808891;
    font: inherit;
  }
  .queue-card,
  .capability-card {
    width: 236px;
    display: grid;
    padding: 19px;
    border-radius: 15px;
  }
  .queue-card {
    top: 15%;
    left: 3%;
  }
  .capability-card {
    right: auto;
    bottom: 13%;
    left: 38%;
  }
  .queue-card span,
  .capability-card span {
    color: #71adff;
    font: 9px var(--font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .queue-card span i,
  .capability-card span i {
    width: 6px;
    height: 6px;
    display: inline-block;
    margin-right: 6px;
    border-radius: 50%;
    background: #5ca3ff;
  }
  .capability-card span i {
    background: #fff;
    box-shadow: 0 0 10px rgb(92 163 255 / 0.8);
  }
  .queue-card strong,
  .capability-card strong {
    margin-top: 12px;
    font-family: 'Instrument Sans Variable', 'Inter Variable', sans-serif;
    font-size: 13px;
    font-variation-settings: 'wdth' 100;
  }
  .queue-card small,
  .capability-card small {
    margin-top: 7px;
    color: #8d9bad;
    font: 10px/1.5 var(--font-mono);
  }
  figcaption {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  @media (max-width: 760px) {
    .render { min-height: 1050px; }
    .queue-card,
    .capability-card {
      width: 190px;
      padding: 13px;
    }
    .queue-card {
      top: 9%;
      left: 2%;
    }
    .capability-card {
      right: 2%;
      bottom: 5%;
      left: auto;
    }
  }
  @media (max-width: 460px) {
    .render { min-height: 1050px; }
    .demo-label {
      bottom: 12px;
      left: 12px;
      white-space: nowrap;
    }
    .demo-label small { display: none; }
    .queue-card { display: none; }
    .capability-card {
      top: 14px;
      right: 12px;
      bottom: auto;
      width: 190px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    canvas { transition: none; }
  }
</style>
