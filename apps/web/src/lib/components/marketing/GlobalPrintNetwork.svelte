<script lang="ts">
  import { onMount } from 'svelte';
  import { NATURAL_EARTH_CITY_LIGHTS } from '$lib/data/natural-earth-city-lights';

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
      cloud: { label: 'Piqae Cloud · US', latitude: 45.52, longitude: -122.68 },
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
      cloud: { label: 'Piqae Cloud · EU', latitude: 50.11, longitude: 8.68 },
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
      cloud: { label: 'Piqae Cloud · APAC', latitude: -33.87, longitude: 151.21 },
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
      cloud: { label: 'Piqae Cloud · US', latitude: 45.52, longitude: -122.68 },
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
      cloud: { label: 'Piqae Cloud · APAC', latitude: 1.35, longitude: 103.82 },
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
      cloud: { label: 'Piqae Cloud · EU', latitude: 50.11, longitude: 8.68 },
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
      cloud: { label: 'Piqae Cloud · APAC', latitude: -33.87, longitude: 151.21 },
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
      cloud: { label: 'Piqae Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Lisbon',
        printer: 'Dispatch station 06',
        latitude: 38.72,
        longitude: -9.14
      },
      format: 'PDF',
      elapsed: '1.6s'
    },
    {
      id: 'job_5H9',
      origin: { label: 'New York', latitude: 40.71, longitude: -74.01 },
      cloud: { label: 'Piqae Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Chicago',
        printer: 'Dispatch station 09',
        latitude: 41.88,
        longitude: -87.63
      },
      format: 'RAW',
      elapsed: '0.8s'
    },
    {
      id: 'job_2B4',
      origin: { label: 'Seattle', latitude: 47.61, longitude: -122.33 },
      cloud: { label: 'Piqae Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Denver',
        printer: 'Production station 05',
        latitude: 39.74,
        longitude: -104.99
      },
      format: 'PDF',
      elapsed: '0.9s'
    },
    {
      id: 'job_6D1',
      origin: { label: 'Madrid', latitude: 40.42, longitude: -3.7 },
      cloud: { label: 'Piqae Cloud · EU', latitude: 50.11, longitude: 8.68 },
      destination: {
        label: 'Stockholm',
        printer: 'Packing station 07',
        latitude: 59.33,
        longitude: 18.07
      },
      format: 'PDF',
      elapsed: '1.0s'
    },
    {
      id: 'job_8J6',
      origin: { label: 'Lagos', latitude: 6.45, longitude: 3.39 },
      cloud: { label: 'Piqae Cloud · EU', latitude: 50.11, longitude: 8.68 },
      destination: {
        label: 'Cape Town',
        printer: 'Customer counter 01',
        latitude: -33.92,
        longitude: 18.42
      },
      format: 'RAW',
      elapsed: '1.3s'
    },
    {
      id: 'job_3T7',
      origin: { label: 'Tokyo', latitude: 35.68, longitude: 139.76 },
      cloud: { label: 'Piqae Cloud · APAC', latitude: -33.87, longitude: 151.21 },
      destination: {
        label: 'Osaka',
        printer: 'Fulfilment station 02',
        latitude: 34.69,
        longitude: 135.5
      },
      format: 'PDF',
      elapsed: '0.7s'
    },
    {
      id: 'job_4W2',
      origin: { label: 'Bangkok', latitude: 13.75, longitude: 100.5 },
      cloud: { label: 'Piqae Cloud · APAC', latitude: 1.35, longitude: 103.82 },
      destination: {
        label: 'Manila',
        printer: 'Label station 14',
        latitude: 14.6,
        longitude: 120.98
      },
      format: 'RAW',
      elapsed: '0.8s'
    },
    {
      id: 'job_9L5',
      origin: { label: 'Mexico City', latitude: 19.43, longitude: -99.13 },
      cloud: { label: 'Piqae Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Bogotá',
        printer: 'Dispatch station 03',
        latitude: 4.71,
        longitude: -74.07
      },
      format: 'PDF',
      elapsed: '1.1s'
    },
    {
      id: 'job_7Q8',
      origin: { label: 'Buenos Aires', latitude: -34.6, longitude: -58.38 },
      cloud: { label: 'Piqae Cloud · US', latitude: 45.52, longitude: -122.68 },
      destination: {
        label: 'Santiago',
        printer: 'Packing station 12',
        latitude: -33.45,
        longitude: -70.67
      },
      format: 'RAW',
      elapsed: '1.2s'
    }
  ];

  let {
    routes = demoRoutes,
    simulated = true,
    solarLighting = true,
    solarStrength = 1,
    oceanTextureStrength = 1
  }: {
    routes?: PrintRoute[];
    simulated?: boolean;
    solarLighting?: boolean;
    solarStrength?: number;
    oceanTextureStrength?: number;
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
              1.68,
              0.56,
              0.82
            );
            composer.addPass(bloomPass);

            const world = new THREE.Group();
            world.rotation.set(-0.12, -0.1, 0.02);
            scene.add(world);

            const globeMaterial = new THREE.ShaderMaterial({
                uniforms: {
                  oceanDark: { value: new THREE.Color(0x010513) },
                  oceanLight: { value: new THREE.Color(0x10123b) },
                  oceanDay: { value: new THREE.Color(0x07506d) },
                  rimColor: { value: new THREE.Color(0x087c8c) },
                  sunDirection: { value: new THREE.Vector3(1, 0, 0) },
                  solarEnabled: { value: solarLighting ? 1 : 0 },
                  solarStrength: {
                    value: Math.max(0, Math.min(1.5, solarStrength))
                  },
                  oceanTextureStrength: {
                    value: Math.max(0, Math.min(1.5, oceanTextureStrength))
                  }
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
                  uniform vec3 oceanDay;
                  uniform vec3 rimColor;
                  uniform vec3 sunDirection;
                  uniform float solarEnabled;
                  uniform float solarStrength;
                  uniform float oceanTextureStrength;
                  varying vec3 vNormal;
                  varying vec3 vViewPosition;
                  varying vec3 vObjectPosition;

                  float random(vec2 coordinate) {
                    return fract(sin(dot(coordinate, vec2(12.9898, 78.233))) * 43758.5453);
                  }

                  float waterNoise(vec3 coordinate) {
                    vec3 cell = fract(coordinate * 0.1031);
                    cell += dot(cell, cell.yzx + 33.33);
                    return fract((cell.x + cell.y) * cell.z);
                  }

                  void main() {
                    vec3 normal = normalize(vNormal);
                    vec3 viewDirection = normalize(vViewPosition);
                    vec3 lightDirection = normalize(vec3(-0.65, 0.48, 0.78));
                    float diffuse = max(dot(normal, lightDirection), 0.0);
                    float sunFacing = dot(
                      normalize(vObjectPosition),
                      normalize(sunDirection)
                    );
                    float daylight = smoothstep(0.12, 0.82, sunFacing);
                    float sunLift = smoothstep(0.52, 0.97, sunFacing);
                    float sunCore = smoothstep(0.72, 0.995, sunFacing);
                    float fresnel = pow(1.0 - max(dot(normal, viewDirection), 0.0), 2.4);
                    float latitudeShade = smoothstep(-1.65, 1.6, vObjectPosition.y);
                    float grain = random(
                      vObjectPosition.xy * 913.7 +
                      vec2(vObjectPosition.z * 417.3, vObjectPosition.z * 271.9)
                    ) - 0.5;
                    float mediumGrain =
                      waterNoise(floor(vObjectPosition * 92.0)) - 0.5;
                    float fineGrain =
                      waterNoise(vObjectPosition * 1850.0) - 0.5;
                    float cyanFleck = smoothstep(
                      0.72,
                      0.98,
                      waterNoise(vObjectPosition * 2670.0)
                    );

                    vec3 ambientOcean = mix(
                      oceanDark,
                      oceanLight,
                      diffuse * 0.7 + latitudeShade * 0.08
                    );
                    vec3 solarOcean = mix(
                      oceanDark * 0.72,
                      oceanDay,
                      daylight * (0.68 + sunLift * 0.32)
                    );
                    solarOcean += rimColor * sunCore * 0.16;
                    float appliedSolar = clamp(
                      solarEnabled * solarStrength,
                      0.0,
                      1.0
                    );
                    vec3 ocean = mix(ambientOcean, solarOcean, appliedSolar);
                    ocean += rimColor * fresnel * 0.19;
                    ocean += grain * 0.018;
                    ocean +=
                      mix(oceanLight, rimColor, daylight * 0.72) *
                      (mediumGrain * 0.13 + fineGrain * 0.09) *
                      oceanTextureStrength;
                    ocean +=
                      rimColor *
                      cyanFleck *
                      (0.018 + daylight * 0.026) *
                      oceanTextureStrength;
                    gl_FragColor = vec4(ocean, 1.0);
                  }
                `
              });
            const globe = new THREE.Mesh(
              new THREE.SphereGeometry(1.72, 96, 96),
              globeMaterial
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
            const landMaskTexture = new THREE.CanvasTexture(maskCanvas);
            landMaskTexture.anisotropy = Math.min(
              4,
              renderer.capabilities.getMaxAnisotropy()
            );
            landMaskTexture.minFilter = THREE.LinearMipmapLinearFilter;
            landMaskTexture.magFilter = THREE.LinearFilter;

            const landSurfaceMaterial = new THREE.ShaderMaterial({
                transparent: true,
                depthWrite: false,
                uniforms: {
                  landMask: { value: landMaskTexture },
                  landColor: { value: new THREE.Color(0x043b34) },
                  landHighlight: { value: new THREE.Color(0x0d705e) },
                  sunDirection: { value: new THREE.Vector3(1, 0, 0) },
                  solarEnabled: { value: solarLighting ? 1 : 0 },
                  solarStrength: {
                    value: Math.max(0, Math.min(1.5, solarStrength))
                  }
                },
                vertexShader: `
                  varying vec2 vUv;
                  varying vec3 vNormal;
                  varying vec3 vObjectPosition;
                  void main() {
                    vUv = uv;
                    vNormal = normalize(normalMatrix * normal);
                    vObjectPosition = position;
                    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
                  }
                `,
                fragmentShader: `
                  uniform sampler2D landMask;
                  uniform vec3 landColor;
                  uniform vec3 landHighlight;
                  uniform vec3 sunDirection;
                  uniform float solarEnabled;
                  uniform float solarStrength;
                  varying vec2 vUv;
                  varying vec3 vNormal;
                  varying vec3 vObjectPosition;
                  void main() {
                    float land = texture2D(landMask, vUv).a;
                    if (land < 0.02) discard;
                    vec3 lightDirection = normalize(vec3(-0.65, 0.48, 0.78));
                    float diffuse = max(dot(normalize(vNormal), lightDirection), 0.0);
                    float sunFacing = dot(
                      normalize(vObjectPosition),
                      normalize(sunDirection)
                    );
                    float daylight = smoothstep(0.12, 0.78, sunFacing);
                    float appliedSolar = clamp(
                      solarEnabled * solarStrength,
                      0.0,
                      1.0
                    );
                    float lightAmount = mix(
                      diffuse * 0.42,
                      daylight * 0.72,
                      appliedSolar
                    );
                    vec3 color = mix(landColor, landHighlight, lightAmount);
                    float solarAlpha = mix(0.2, 0.34, daylight);
                    float alpha = mix(0.28, solarAlpha, appliedSolar);
                    gl_FragColor = vec4(color, land * alpha);
                  }
                `
              });
            const landSurface = new THREE.Mesh(
              new THREE.SphereGeometry(1.731, 96, 96),
              landSurfaceMaterial
            );
            world.add(landSurface);

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
            for (let latitude = -60; latitude <= 84; latitude += 0.36) {
              for (let longitude = -179.5; longitude <= 179.5; longitude += 0.4) {
                if (!isLand(latitude, longitude)) continue;
                const latitudeJitter = Math.sin(longitude * 4.21 + latitude * 1.77) * 0.1;
                const longitudeJitter = Math.cos(latitude * 3.93 - longitude * 1.31) * 0.1;
                const point = latitudeLongitudeToVector(
                  latitude + latitudeJitter,
                  longitude + longitudeJitter,
                  1.739
                );
                landPositions.push(point.x, point.y, point.z);
              }
            }

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
              makeDotMaterial(0x24b98f, 4.9, 0.84, false, 0.78)
            );
            world.add(landPoints);

            const cityLightPositions: number[] = [];
            const cityLightScales: number[] = [];
            const cityLightPhases: number[] = [];
            NATURAL_EARTH_CITY_LIGHTS.forEach(
              ([latitude, longitude, population], cityIndex) => {
                const populationWeight = Math.min(
                  1.25,
                  Math.max(0.28, (Math.log10(population) - 5) * 0.5)
                );
                const satelliteCount =
                  population >= 1000000
                    ? 1 + Math.round(populationWeight * 3)
                    : population >= 500000
                      ? 1
                      : 0;
                const longitudeScale = Math.max(
                  0.35,
                  Math.cos((latitude * Math.PI) / 180)
                );

                for (let lightIndex = 0; lightIndex <= satelliteCount; lightIndex += 1) {
                  const seed = Math.abs(
                    Math.sin(
                      (cityIndex + 1) * 91.733 +
                      (lightIndex + 1) * 17.171
                    )
                  );
                  const spreadSeed = Math.abs(
                    Math.sin(
                      (cityIndex + 1) * 37.719 +
                      (lightIndex + 1) * 113.137
                    )
                  );
                  const angle =
                    seed * Math.PI * 2 +
                    lightIndex * 2.399963229728653;
                  const distance =
                    lightIndex === 0
                      ? 0
                      : (0.06 + Math.pow(spreadSeed, 1.7) * 0.46) *
                        (0.72 + populationWeight * 0.32);
                  const light = latitudeLongitudeToVector(
                    latitude + Math.cos(angle) * distance,
                    longitude + (Math.sin(angle) * distance) / longitudeScale,
                    1.754
                  );
                  cityLightPositions.push(light.x, light.y, light.z);
                  cityLightScales.push(
                    lightIndex === 0
                      ? 0.42 + populationWeight * 0.5
                      : populationWeight * (0.18 + seed * 0.28)
                  );
                  cityLightPhases.push(
                    (cityIndex * 0.73 + lightIndex * 1.91) % (Math.PI * 2)
                  );
                }
              }
            );

            const lightGeometry = new THREE.BufferGeometry();
            lightGeometry.setAttribute(
              'position',
              new THREE.Float32BufferAttribute(cityLightPositions, 3)
            );
            lightGeometry.setAttribute(
              'cityScale',
              new THREE.Float32BufferAttribute(cityLightScales, 1)
            );
            lightGeometry.setAttribute(
              'cityPhase',
              new THREE.Float32BufferAttribute(cityLightPhases, 1)
            );

            const cityLightMaterial = new THREE.ShaderMaterial({
              transparent: true,
              depthWrite: false,
              blending: THREE.AdditiveBlending,
              toneMapped: false,
              uniforms: {
                dotColor: { value: new THREE.Color(0xffbd73) },
                dotSize: { value: 5.1 * renderer.getPixelRatio() },
                dotOpacity: { value: 0.96 },
                sunDirection: { value: new THREE.Vector3(1, 0, 0) },
                solarEnabled: { value: solarLighting ? 1 : 0 },
                solarStrength: {
                  value: Math.max(0, Math.min(1.5, solarStrength))
                },
                cityTime: { value: 0 }
              },
              vertexShader: `
                attribute float cityScale;
                attribute float cityPhase;
                uniform float dotSize;
                uniform vec3 sunDirection;
                uniform float solarEnabled;
                uniform float solarStrength;
                varying float vNight;
                varying float vIntensity;
                varying float vPhase;
                void main() {
                  vec3 surfaceNormal = normalize(position);
                  float sunlight = dot(surfaceNormal, normalize(sunDirection));
                  float nightMask = 1.0 - smoothstep(-0.22, 0.12, sunlight);
                  float solarNight = 0.42 + nightMask * 0.58;
                  float appliedSolar = clamp(
                    solarEnabled * solarStrength,
                    0.0,
                    1.0
                  );
                  vNight = mix(1.0, solarNight, appliedSolar);
                  vIntensity = cityScale;
                  vPhase = cityPhase;
                  vec4 viewPosition = modelViewMatrix * vec4(position, 1.0);
                  float perspectiveScale = 4.6 / max(3.2, -viewPosition.z);
                  gl_PointSize = dotSize * cityScale * perspectiveScale;
                  gl_Position = projectionMatrix * viewPosition;
                }
              `,
              fragmentShader: `
                uniform vec3 dotColor;
                uniform float dotOpacity;
                uniform float cityTime;
                varying float vNight;
                varying float vIntensity;
                varying float vPhase;
                void main() {
                  float distanceFromCenter = length(gl_PointCoord - vec2(0.5)) * 2.0;
                  if (distanceFromCenter > 1.0) discard;
                  float halo = exp(-3.8 * distanceFromCenter * distanceFromCenter);
                  float core = smoothstep(0.32, 0.0, distanceFromCenter);
                  float shimmer = 0.97 + sin(cityTime * 0.72 + vPhase) * 0.03;
                  float alpha =
                    (halo * 0.68 + core * 1.08) * dotOpacity * vNight * shimmer;
                  gl_FragColor = vec4(
                    dotColor * (1.7 + vIntensity * 0.68),
                    alpha
                  );
                }
              `
            });
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

            const makeSparkMaterial = (phase: number) =>
              new THREE.ShaderMaterial({
                transparent: true,
                depthWrite: false,
                blending: THREE.AdditiveBlending,
                toneMapped: false,
                uniforms: {
                  sparkColor: { value: new THREE.Color(0xeaf5ff) },
                  sparkSize: { value: 8.2 * renderer.getPixelRatio() },
                  sparkHead: { value: -1 },
                  sparkTail: { value: 0.42 },
                  sparkOpacity: { value: 0 },
                  sparkTime: { value: 0 },
                  sparkPhase: { value: phase }
                },
                vertexShader: `
                  attribute float routeProgress;
                  uniform float sparkSize;
                  varying float vProgress;
                  varying float vNoise;
                  void main() {
                    vProgress = routeProgress;
                    vNoise = fract(
                      sin(dot(position.xy + position.zz, vec2(57.17, 193.41))) *
                      19417.37
                    );
                    vec4 viewPosition = modelViewMatrix * vec4(position, 1.0);
                    gl_PointSize =
                      sparkSize * (0.42 + vNoise * 0.68) /
                      max(0.7, -viewPosition.z);
                    gl_Position = projectionMatrix * viewPosition;
                  }
                `,
                fragmentShader: `
                  uniform vec3 sparkColor;
                  uniform float sparkHead;
                  uniform float sparkTail;
                  uniform float sparkOpacity;
                  uniform float sparkTime;
                  uniform float sparkPhase;
                  varying float vProgress;
                  varying float vNoise;
                  void main() {
                    float distanceFromCenter = length(gl_PointCoord - vec2(0.5)) * 2.0;
                    if (distanceFromCenter > 1.0) discard;
                    float behind = sparkHead - vProgress;
                    float inTrail =
                      step(0.0, behind) *
                      (1.0 - smoothstep(sparkTail * 0.46, sparkTail, behind));
                    float trailTaper =
                      0.22 + 0.78 * (1.0 - clamp(behind / sparkTail, 0.0, 1.0));
                    float twinkle =
                      0.72 +
                      sin(
                        sparkTime * (5.2 + vNoise * 2.6) +
                        sparkPhase +
                        vNoise * 18.849
                      ) * 0.28;
                    float halo = exp(-4.8 * distanceFromCenter * distanceFromCenter);
                    float core = smoothstep(0.34, 0.0, distanceFromCenter);
                    float alpha =
                      (halo * 0.52 + core) *
                      inTrail *
                      trailTaper *
                      twinkle *
                      sparkOpacity;
                    gl_FragColor = vec4(
                      sparkColor * (2.05 + twinkle * 1.35),
                      alpha
                    );
                  }
                `
              });

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
              const inboundPoints = inbound.getPoints(144);
              const outboundPoints = outbound.getPoints(144);
              const inboundGeometry = new THREE.BufferGeometry().setFromPoints(inboundPoints);
              const outboundGeometry = new THREE.BufferGeometry().setFromPoints(outboundPoints);
              const inboundSparkGeometry = new THREE.BufferGeometry().setFromPoints(inboundPoints);
              const outboundSparkGeometry = new THREE.BufferGeometry().setFromPoints(outboundPoints);
              inboundSparkGeometry.setAttribute(
                'routeProgress',
                new THREE.Float32BufferAttribute(
                  inboundPoints.map((_, pointIndex) => pointIndex / (inboundPoints.length - 1)),
                  1
                )
              );
              outboundSparkGeometry.setAttribute(
                'routeProgress',
                new THREE.Float32BufferAttribute(
                  outboundPoints.map((_, pointIndex) => pointIndex / (outboundPoints.length - 1)),
                  1
                )
              );
              inboundGeometry.setDrawRange(0, 0);
              outboundGeometry.setDrawRange(0, 0);
              const lineMaterials = [
                new THREE.LineBasicMaterial({
                  color: new THREE.Color(0xcfe6ff).multiplyScalar(3.45),
                  transparent: true,
                  opacity: 0,
                  blending: THREE.AdditiveBlending,
                  depthWrite: false,
                  toneMapped: false
                }),
                new THREE.LineBasicMaterial({
                  color: new THREE.Color(0xeaf4ff).multiplyScalar(3.7),
                  transparent: true,
                  opacity: 0,
                  blending: THREE.AdditiveBlending,
                  depthWrite: false,
                  toneMapped: false
                })
              ];

              group.add(
                new THREE.Line(inboundGeometry, lineMaterials[0]),
                new THREE.Line(outboundGeometry, lineMaterials[1])
              );

              const sparkMaterials = [
                makeSparkMaterial(index * 0.91),
                makeSparkMaterial(index * 0.91 + 2.3)
              ];
              group.add(
                new THREE.Points(inboundSparkGeometry, sparkMaterials[0]),
                new THREE.Points(outboundSparkGeometry, sparkMaterials[1])
              );

              const nodes = [
                [origin, 0xd9ecff, 0.011],
                [cloud, 0x5ca3ff, 0.014],
                [destination, 0xffdf9c, 0.012]
              ] as const;
              const nodeMaterials: InstanceType<typeof THREE.MeshBasicMaterial>[] = [];
              nodes.forEach(([position, color, size]) => {
                const material = new THREE.MeshBasicMaterial({
                  color: new THREE.Color(color).multiplyScalar(1.3),
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
                color: new THREE.Color(0xeaf4ff).multiplyScalar(1.65),
                transparent: true,
                opacity: 0,
                blending: THREE.AdditiveBlending,
                depthWrite: false
              });
              const particle = new THREE.Mesh(
                new THREE.SphereGeometry(0.009, 10, 10),
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
                sparkMaterials,
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
            let lastSunMinute = -1;
            const syncSunDirection = () => {
              const now = new Date();
              const minuteKey = Math.floor(now.getTime() / 60000);
              if (minuteKey === lastSunMinute) return;
              lastSunMinute = minuteKey;

              const yearStart = Date.UTC(now.getUTCFullYear(), 0, 1);
              const today = Date.UTC(
                now.getUTCFullYear(),
                now.getUTCMonth(),
                now.getUTCDate()
              );
              const dayOfYear = Math.floor((today - yearStart) / 86400000) + 1;
              const utcMinutes =
                now.getUTCHours() * 60 +
                now.getUTCMinutes() +
                now.getUTCSeconds() / 60;
              const fractionalYear =
                (2 * Math.PI * (dayOfYear - 1 + (utcMinutes / 60 - 12) / 24)) /
                365;
              const equationOfTime =
                229.18 *
                (0.000075 +
                  0.001868 * Math.cos(fractionalYear) -
                  0.032077 * Math.sin(fractionalYear) -
                  0.014615 * Math.cos(2 * fractionalYear) -
                  0.040849 * Math.sin(2 * fractionalYear));
              const declination =
                0.006918 -
                0.399912 * Math.cos(fractionalYear) +
                0.070257 * Math.sin(fractionalYear) -
                0.006758 * Math.cos(2 * fractionalYear) +
                0.000907 * Math.sin(2 * fractionalYear) -
                0.002697 * Math.cos(3 * fractionalYear) +
                0.00148 * Math.sin(3 * fractionalYear);
              const subsolarLatitude = (declination * 180) / Math.PI;
              const rawLongitude = (720 - utcMinutes - equationOfTime) / 4;
              const subsolarLongitude = ((rawLongitude + 540) % 360) - 180;
              const direction = latitudeLongitudeToVector(
                subsolarLatitude,
                subsolarLongitude,
                1
              ).normalize();
              globeMaterial.uniforms.sunDirection!.value.copy(direction);
              landSurfaceMaterial.uniforms.sunDirection!.value.copy(direction);
              cityLightMaterial.uniforms.sunDirection!.value.copy(direction);
            };
            syncSunDirection();

            const routeStagger = 0.27;
            const routeLifetime = 2.35;
            const routeCycle =
              Math.max(routeSequence.length, 1) * routeStagger + routeLifetime + 0.38;

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
                    material.opacity = shown ? 0.34 : 0;
                  });
                  visual.sparkMaterials.forEach((material) => {
                    material.uniforms.sparkOpacity!.value = 0;
                  });
                  visual.nodeMaterials.forEach((material) => {
                    material.opacity = shown ? 0.42 : 0;
                  });
                  visual.particle.visible = false;
                  return;
                }

                const start = visual.sequencePosition * routeStagger;
                const localTime = ((elapsed - start) % routeCycle + routeCycle) % routeCycle;
                const inboundProgress = smoothstep(0, 0.48, localTime);
                const outboundProgress = smoothstep(0.35, 0.94, localTime);
                const coreFade = 1 - smoothstep(1.05, 1.63, localTime);
                const coreOpacity = Math.pow(coreFade, 1.32);
                const lineVisible = coreOpacity > 0.055;
                const visible = localTime <= routeLifetime;
                const opacity = visible && lineVisible ? coreOpacity : 0;

                visual.inboundGeometry.setDrawRange(
                  0,
                  opacity > 0
                    ? Math.ceil(visual.inboundPointCount * inboundProgress)
                    : 0
                );
                visual.outboundGeometry.setDrawRange(
                  0,
                  opacity > 0
                    ? Math.ceil(visual.outboundPointCount * outboundProgress)
                    : 0
                );
                visual.lineMaterials[0]!.opacity = opacity * 0.9;
                visual.lineMaterials[1]!.opacity = opacity * 0.96;
                const inboundSparkFade =
                  visible ? 1 - smoothstep(0.9, 1.7, localTime) : 0;
                const outboundSparkFade =
                  visible ? 1 - smoothstep(1.25, 2.18, localTime) : 0;
                visual.sparkMaterials[0]!.uniforms.sparkHead!.value = inboundProgress;
                visual.sparkMaterials[0]!.uniforms.sparkOpacity!.value =
                  inboundSparkFade * 0.88;
                visual.sparkMaterials[0]!.uniforms.sparkTime!.value = elapsed;
                visual.sparkMaterials[1]!.uniforms.sparkHead!.value = outboundProgress;
                visual.sparkMaterials[1]!.uniforms.sparkOpacity!.value =
                  outboundSparkFade * 0.94;
                visual.sparkMaterials[1]!.uniforms.sparkTime!.value = elapsed;
                const nodeFade = visible
                  ? 1 - smoothstep(1.3, 2.14, localTime)
                  : 0;
                visual.nodeMaterials[0]!.opacity =
                  nodeFade * smoothstep(0, 0.15, localTime) * 0.62;
                visual.nodeMaterials[1]!.opacity =
                  nodeFade * smoothstep(0.32, 0.52, localTime) * 0.7;
                visual.nodeMaterials[2]!.opacity =
                  nodeFade * smoothstep(0.84, 1.04, localTime) * 0.7;

                const drawingInbound = localTime <= 0.48;
                const drawingOutbound = localTime > 0.35 && localTime <= 0.94;
                visual.particle.visible = visible && (drawingInbound || drawingOutbound);
                visual.particleMaterial.opacity = lineVisible ? coreOpacity * 0.72 : 0;
                if (drawingInbound) {
                  visual.particle.position.copy(
                    visual.inbound.getPoint(clamp(localTime / 0.48))
                  );
                } else if (drawingOutbound) {
                  visual.particle.position.copy(
                    visual.outbound.getPoint(clamp((localTime - 0.35) / 0.59))
                  );
                }
                visual.particle.scale.setScalar(1);
              });
            };

            const render = () => composer.render();
            const resize = () => {
              const width = stage.clientWidth;
              const height = stage.clientHeight;
              camera.position.z = width < 600 ? 6.15 : 5.2;
              world.position.x = width < 900 ? 0 : -1.08;
              world.position.y = width < 600 ? -0.82 : -0.08;
              renderer.setSize(width, height, false);
              composer.setSize(width, height);
              camera.aspect = width / Math.max(height, 1);
              camera.updateProjectionMatrix();
              render();
            };

            let previousFrame = performance.now();
            let elapsed = 0;
            let animationVisible = false;
            const animate = (now = performance.now()) => {
              if (!animationVisible || document.visibilityState === 'hidden') {
                animationFrame = 0;
                return;
              }
              const delta = Math.min((now - previousFrame) / 1000, 0.04);
              previousFrame = now;
              elapsed += delta;
              if (!dragging) targetRotationY += delta * 0.075;
              world.rotation.x += (targetRotationX - world.rotation.x) * 0.07;
              world.rotation.y += (targetRotationY - world.rotation.y) * 0.07;

              updateRouteVisuals(elapsed);
              syncSunDirection();
              cityLightMaterial.uniforms.cityTime!.value = elapsed;

              camera.position.x = Math.sin(elapsed * 0.08) * 0.08;
              camera.position.y = 0.05 + Math.cos(elapsed * 0.11) * 0.045;
              camera.lookAt(0, 0, 0);
              starField.rotation.y -= delta * 0.006;
              render();
              animationFrame = requestAnimationFrame(animate);
            };
            const resumeAnimation = () => {
              if (
                reducedMotion ||
                !animationVisible ||
                document.visibilityState === 'hidden' ||
                animationFrame !== 0
              ) {
                return;
              }
              previousFrame = performance.now();
              animationFrame = requestAnimationFrame(animate);
            };
            const pauseAnimation = () => {
              cancelAnimationFrame(animationFrame);
              animationFrame = 0;
            };
            const visibilityObserver = new IntersectionObserver(
              ([entry]) => {
                animationVisible = Boolean(entry?.isIntersecting);
                if (animationVisible) resumeAnimation();
                else pauseAnimation();
              },
              { rootMargin: '120px' }
            );
            const handleDocumentVisibility = () => {
              if (document.visibilityState === 'hidden') pauseAnimation();
              else resumeAnimation();
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
            document.addEventListener('visibilitychange', handleDocumentVisibility);

            const resizeObserver = new ResizeObserver(resize);
            resizeObserver.observe(stage);
            visibilityObserver.observe(stage);
            updateRouteVisuals(0);
            resize();
            ready = true;

            dispose = () => {
              pauseAnimation();
              resizeObserver.disconnect();
              visibilityObserver.disconnect();
              canvas.removeEventListener('pointerdown', handlePointerDown);
              canvas.removeEventListener('pointermove', handlePointerMove);
              canvas.removeEventListener('pointerup', handlePointerUp);
              canvas.removeEventListener('pointercancel', handlePointerUp);
              document.removeEventListener('visibilitychange', handleDocumentVisibility);
              routeVisuals.forEach((visual) => {
                visual.group.traverse((object) => {
                  if (
                    object instanceof THREE.Mesh ||
                    object instanceof THREE.Line ||
                    object instanceof THREE.Points
                  ) {
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
              landSurface.geometry.dispose();
              landSurface.material.dispose();
              landMaskTexture.dispose();
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
    aria-label="Interactive three-dimensional globe illustrating print jobs travelling through Piqae to local printers"
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
  .render::before,
  .render::after {
    position: absolute;
    z-index: 1;
    inset: 0;
    content: '';
    pointer-events: none;
  }
  .render::before {
    background:
      radial-gradient(circle at 29% 45%, rgb(0 60 54 / .08), transparent 49%),
      linear-gradient(
        105deg,
        rgb(0 37 32 / .2),
        rgb(2 11 22 / .03) 55%,
        rgb(0 34 28 / .2)
      );
    mix-blend-mode: color;
  }
  .render::after {
    background:
      radial-gradient(circle at 48% 48%, transparent 34%, rgb(2 11 22 / 0.08) 66%, rgb(2 11 22 / 0.58) 100%);
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
    filter: saturate(.88) contrast(1.08) brightness(.96);
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
